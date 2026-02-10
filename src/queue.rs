use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::{Mutex, Notify};
use tracing::{debug, info, warn};

const MAX_RETRIES: u32 = 5;
const BASE_RETRY_MS: u64 = 5000;

#[derive(Debug, Clone, PartialEq)]
pub enum GroupStatus {
    Idle,
    Running,
    Queued,
}

type ProcessFn = Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

struct GroupState {
    active: bool,
    pending_messages: bool,
    pending_tasks: VecDeque<QueuedTask>,
    retry_count: u32,
}

struct QueuedTask {
    id: String,
    group_jid: String,
    task_fn: Pin<Box<dyn Future<Output = ()> + Send>>,
}

pub struct GroupQueue {
    inner: Arc<Mutex<GroupQueueInner>>,
    shutdown_notify: Arc<Notify>,
}

struct GroupQueueInner {
    groups: HashMap<String, GroupState>,
    active_count: usize,
    max_concurrent: usize,
    waiting_groups: VecDeque<String>,
    process_fn: Option<ProcessFn>,
    shutting_down: bool,
    on_state_change: Option<Box<dyn Fn() + Send + Sync>>,
}

impl GroupQueue {
    pub fn new(max_concurrent: usize) -> Self {
        GroupQueue {
            inner: Arc::new(Mutex::new(GroupQueueInner {
                groups: HashMap::new(),
                active_count: 0,
                max_concurrent,
                waiting_groups: VecDeque::new(),
                process_fn: None,
                shutting_down: false,
                on_state_change: None,
            })),
            shutdown_notify: Arc::new(Notify::new()),
        }
    }

    pub async fn set_process_fn(&self, f: ProcessFn) {
        let mut inner = self.inner.lock().await;
        inner.process_fn = Some(f);
    }

    pub async fn set_on_state_change(&self, f: Box<dyn Fn() + Send + Sync>) {
        let mut inner = self.inner.lock().await;
        inner.on_state_change = Some(f);
    }

    pub async fn get_active_count(&self) -> usize {
        let inner = self.inner.lock().await;
        inner.active_count
    }

    pub async fn get_group_status(&self, group_jid: &str) -> GroupStatus {
        let inner = self.inner.lock().await;
        match inner.groups.get(group_jid) {
            None => GroupStatus::Idle,
            Some(state) => {
                if state.active {
                    GroupStatus::Running
                } else if state.pending_messages || !state.pending_tasks.is_empty() {
                    GroupStatus::Queued
                } else {
                    GroupStatus::Idle
                }
            }
        }
    }

    pub async fn enqueue_message_check(&self, group_jid: String) {
        let mut inner = self.inner.lock().await;
        if inner.shutting_down {
            return;
        }

        // Ensure group entry exists
        inner.groups.entry(group_jid.clone()).or_insert(GroupState {
            active: false,
            pending_messages: false,
            pending_tasks: VecDeque::new(),
            retry_count: 0,
        });

        // Now access via get_mut so the borrow is scoped properly
        let is_active = inner.groups.get(&group_jid).map_or(false, |s| s.active);

        if is_active {
            if let Some(state) = inner.groups.get_mut(&group_jid) {
                state.pending_messages = true;
            }
            debug!(group_jid, "Agent active, message queued");
            return;
        }

        if inner.active_count >= inner.max_concurrent {
            if let Some(state) = inner.groups.get_mut(&group_jid) {
                state.pending_messages = true;
            }
            if !inner.waiting_groups.contains(&group_jid) {
                inner.waiting_groups.push_back(group_jid);
            }
            debug!("At concurrency limit, message queued");
            return;
        }

        // Can run immediately
        let process_fn = inner.process_fn.clone();
        if let Some(state) = inner.groups.get_mut(&group_jid) {
            state.active = true;
            state.pending_messages = false;
        }
        inner.active_count += 1;

        if let Some(ref cb) = inner.on_state_change {
            cb();
        }

        let queue = self.inner.clone();
        let shutdown_notify = self.shutdown_notify.clone();
        let jid = group_jid.clone();

        drop(inner);

        tokio::spawn(async move {
            if let Some(process_fn) = process_fn {
                let success = process_fn(jid.clone()).await;
                let mut inner = queue.lock().await;
                if let Some(state) = inner.groups.get_mut(&jid) {
                    if success {
                        state.retry_count = 0;
                    }
                    state.active = false;
                }
                inner.active_count -= 1;
                if let Some(ref cb) = inner.on_state_change {
                    cb();
                }

                // Drain
                Self::drain_locked(&mut inner, &jid, queue.clone(), shutdown_notify);
            }
        });
    }

    fn drain_locked(
        inner: &mut GroupQueueInner,
        group_jid: &str,
        queue: Arc<Mutex<GroupQueueInner>>,
        shutdown_notify: Arc<Notify>,
    ) {
        if inner.shutting_down {
            if inner.active_count == 0 {
                shutdown_notify.notify_one();
            }
            return;
        }

        if let Some(state) = inner.groups.get_mut(group_jid) {
            // Check for pending tasks first
            if !state.pending_tasks.is_empty() {
                // Tasks would need to be drained here
                // For simplicity, check pending messages
            }

            if state.pending_messages && inner.active_count < inner.max_concurrent {
                let process_fn = inner.process_fn.clone();
                state.active = true;
                state.pending_messages = false;
                inner.active_count += 1;

                if let Some(ref cb) = inner.on_state_change {
                    cb();
                }

                let jid = group_jid.to_string();
                let queue_clone = queue.clone();
                let notify_clone = shutdown_notify.clone();

                tokio::spawn(async move {
                    if let Some(process_fn) = process_fn {
                        let success = process_fn(jid.clone()).await;
                        let mut inner = queue_clone.lock().await;
                        if let Some(state) = inner.groups.get_mut(&jid) {
                            if success {
                                state.retry_count = 0;
                            }
                            state.active = false;
                        }
                        inner.active_count -= 1;
                        if let Some(ref cb) = inner.on_state_change {
                            cb();
                        }
                        Self::drain_locked(&mut inner, &jid, queue_clone.clone(), notify_clone);
                    }
                });

                return;
            }
        }

        // Try drain waiting groups
        while !inner.waiting_groups.is_empty() && inner.active_count < inner.max_concurrent {
            if let Some(next_jid) = inner.waiting_groups.pop_front() {
                if let Some(state) = inner.groups.get_mut(&next_jid) {
                    if state.pending_messages {
                        let process_fn = inner.process_fn.clone();
                        state.active = true;
                        state.pending_messages = false;
                        inner.active_count += 1;

                        if let Some(ref cb) = inner.on_state_change {
                            cb();
                        }

                        let jid = next_jid.clone();
                        let queue_clone = queue.clone();
                        let notify_clone = shutdown_notify.clone();

                        tokio::spawn(async move {
                            if let Some(process_fn) = process_fn {
                                let success = process_fn(jid.clone()).await;
                                let mut inner = queue_clone.lock().await;
                                if let Some(state) = inner.groups.get_mut(&jid) {
                                    if success {
                                        state.retry_count = 0;
                                    }
                                    state.active = false;
                                }
                                inner.active_count -= 1;
                                if let Some(ref cb) = inner.on_state_change {
                                    cb();
                                }
                                Self::drain_locked(
                                    &mut inner,
                                    &jid,
                                    queue_clone.clone(),
                                    notify_clone,
                                );
                            }
                        });
                    }
                }
            }
        }
    }

    pub async fn shutdown(&self, grace_period_ms: u64) {
        {
            let mut inner = self.inner.lock().await;
            inner.shutting_down = true;
            info!(
                active_count = inner.active_count,
                "GroupQueue shutting down"
            );

            if inner.active_count == 0 {
                return;
            }
        }

        // Wait for active agents to complete
        let timeout = tokio::time::Duration::from_millis(grace_period_ms);
        let _ = tokio::time::timeout(timeout, self.shutdown_notify.notified()).await;

        let inner = self.inner.lock().await;
        if inner.active_count > 0 {
            warn!(
                active_count = inner.active_count,
                "Some agents did not complete within grace period"
            );
        }
    }
}
