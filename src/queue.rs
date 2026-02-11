use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::{Mutex, Notify};
use tracing::{debug, info, warn};

type ProcessFn = Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

struct ModuleState {
    active: bool,
    pending: bool,
}

pub struct ModuleQueue {
    inner: Arc<Mutex<ModuleQueueInner>>,
    shutdown_notify: Arc<Notify>,
}

struct ModuleQueueInner {
    modules: HashMap<String, ModuleState>,
    active_count: usize,
    max_concurrent: usize,
    waiting_modules: VecDeque<String>,
    process_fn: Option<ProcessFn>,
    shutting_down: bool,
    on_state_change: Option<Box<dyn Fn(usize, usize) + Send + Sync>>,
}

impl ModuleQueue {
    pub fn new(max_concurrent: usize) -> Self {
        ModuleQueue {
            inner: Arc::new(Mutex::new(ModuleQueueInner {
                modules: HashMap::new(),
                active_count: 0,
                max_concurrent,
                waiting_modules: VecDeque::new(),
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

    pub async fn set_on_state_change(&self, f: Box<dyn Fn(usize, usize) + Send + Sync>) {
        let mut inner = self.inner.lock().await;
        inner.on_state_change = Some(f);
    }

    pub async fn get_active_count(&self) -> usize {
        let inner = self.inner.lock().await;
        inner.active_count
    }

    pub async fn enqueue_module(&self, module_id: String) {
        let mut inner = self.inner.lock().await;
        if inner.shutting_down {
            return;
        }

        inner
            .modules
            .entry(module_id.clone())
            .or_insert(ModuleState {
                active: false,
                pending: false,
            });

        let is_active = inner.modules.get(&module_id).is_some_and(|s| s.active);

        if is_active {
            if let Some(state) = inner.modules.get_mut(&module_id) {
                state.pending = true;
            }
            debug!(module_id, "Module active, message queued");
            return;
        }

        if inner.active_count >= inner.max_concurrent {
            if let Some(state) = inner.modules.get_mut(&module_id) {
                state.pending = true;
            }
            if !inner.waiting_modules.contains(&module_id) {
                inner.waiting_modules.push_back(module_id);
            }
            debug!("At concurrency limit, message queued");
            return;
        }

        let process_fn = inner.process_fn.clone();
        if let Some(state) = inner.modules.get_mut(&module_id) {
            state.active = true;
            state.pending = false;
        }
        inner.active_count += 1;

        if let Some(ref cb) = inner.on_state_change {
            let pending_count = inner.modules.values().filter(|s| s.pending).count();
            cb(inner.active_count, inner.active_count + pending_count);
        }

        let queue = self.inner.clone();
        let shutdown_notify = self.shutdown_notify.clone();
        let module_key = module_id.clone();

        drop(inner);

        tokio::spawn(async move {
            if let Some(process_fn) = process_fn {
                let _success = process_fn(module_key.clone()).await;
                let mut inner = queue.lock().await;
                if let Some(state) = inner.modules.get_mut(&module_key) {
                    state.active = false;
                }
                inner.active_count = inner.active_count.saturating_sub(1);
                if let Some(ref cb) = inner.on_state_change {
                    let pending_count = inner.modules.values().filter(|s| s.pending).count();
                    cb(inner.active_count, inner.active_count + pending_count);
                }

                Self::drain_locked(&mut inner, &module_key, queue.clone(), shutdown_notify);
            }
        });
    }

    fn drain_locked(
        inner: &mut ModuleQueueInner,
        module_id: &str,
        queue: Arc<Mutex<ModuleQueueInner>>,
        shutdown_notify: Arc<Notify>,
    ) {
        if inner.shutting_down {
            if inner.active_count == 0 {
                shutdown_notify.notify_one();
            }
            return;
        }

        if let Some(state) = inner.modules.get_mut(module_id) {
            if state.pending && inner.active_count < inner.max_concurrent {
                let process_fn = inner.process_fn.clone();
                state.active = true;
                state.pending = false;
                inner.active_count += 1;

                if let Some(ref cb) = inner.on_state_change {
                    let pending_count = inner.modules.values().filter(|s| s.pending).count();
                    cb(inner.active_count, inner.active_count + pending_count);
                }

                let module_key = module_id.to_string();
                let queue_clone = queue.clone();
                let notify_clone = shutdown_notify.clone();

                tokio::spawn(async move {
                    if let Some(process_fn) = process_fn {
                        let _success = process_fn(module_key.clone()).await;
                        let mut inner = queue_clone.lock().await;
                        if let Some(state) = inner.modules.get_mut(&module_key) {
                            state.active = false;
                        }
                        inner.active_count = inner.active_count.saturating_sub(1);
                        if let Some(ref cb) = inner.on_state_change {
                            let pending_count =
                                inner.modules.values().filter(|s| s.pending).count();
                            cb(inner.active_count, inner.active_count + pending_count);
                        }
                        Self::drain_locked(
                            &mut inner,
                            &module_key,
                            queue_clone.clone(),
                            notify_clone,
                        );
                    }
                });

                return;
            }
        }

        while !inner.waiting_modules.is_empty() && inner.active_count < inner.max_concurrent {
            if let Some(next_module) = inner.waiting_modules.pop_front() {
                if let Some(state) = inner.modules.get_mut(&next_module) {
                    if state.pending {
                        let process_fn = inner.process_fn.clone();
                        state.active = true;
                        state.pending = false;
                        inner.active_count += 1;

                        if let Some(ref cb) = inner.on_state_change {
                            let pending_count =
                                inner.modules.values().filter(|s| s.pending).count();
                            cb(inner.active_count, inner.active_count + pending_count);
                        }

                        let module_key = next_module.clone();
                        let queue_clone = queue.clone();
                        let notify_clone = shutdown_notify.clone();

                        tokio::spawn(async move {
                            if let Some(process_fn) = process_fn {
                                let _success = process_fn(module_key.clone()).await;
                                let mut inner = queue_clone.lock().await;
                                if let Some(state) = inner.modules.get_mut(&module_key) {
                                    state.active = false;
                                }
                                inner.active_count = inner.active_count.saturating_sub(1);
                                if let Some(ref cb) = inner.on_state_change {
                                    let pending_count =
                                        inner.modules.values().filter(|s| s.pending).count();
                                    cb(inner.active_count, inner.active_count + pending_count);
                                }
                                Self::drain_locked(
                                    &mut inner,
                                    &module_key,
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
                "ModuleQueue shutting down"
            );

            if inner.active_count == 0 {
                return;
            }
        }

        let timeout = tokio::time::Duration::from_millis(grace_period_ms);
        let _ = tokio::time::timeout(timeout, self.shutdown_notify.notified()).await;

        let inner = self.inner.lock().await;
        if inner.active_count > 0 {
            warn!(
                active_count = inner.active_count,
                "Some modules did not complete within grace period"
            );
        }
    }
}
