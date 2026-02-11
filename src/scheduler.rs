use std::sync::Arc;

use chrono::Utc;
use cron::Schedule;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::db::Database;
use crate::types::{RunStatus, ScheduleType, ScheduledTask, TaskRunLog, TaskStatus};

pub struct Scheduler {
    db: Arc<Database>,
    config: Arc<Config>,
    task_tx: mpsc::Sender<ScheduledTask>,
    running: bool,
}

impl Scheduler {
    pub fn new(
        db: Arc<Database>,
        config: Arc<Config>,
        task_tx: mpsc::Sender<ScheduledTask>,
    ) -> Self {
        Scheduler {
            db,
            config,
            task_tx,
            running: false,
        }
    }

    pub async fn run_loop(&mut self, mut shutdown_rx: tokio::sync::watch::Receiver<bool>) {
        self.running = true;
        info!("Scheduler loop started");

        loop {
            tokio::select! {
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(
                    self.config.scheduler_poll_interval_ms
                )) => {
                    self.check_due_tasks().await;
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        info!("Scheduler loop stopped");
                        break;
                    }
                }
            }
        }

        self.running = false;
    }

    async fn check_due_tasks(&self) {
        let due_tasks = match self.db.get_due_tasks() {
            Ok(tasks) => tasks,
            Err(e) => {
                error!("Failed to get due tasks: {}", e);
                return;
            }
        };

        if !due_tasks.is_empty() {
            info!(count = due_tasks.len(), "Found due tasks");
        }

        for task in due_tasks {
            // Re-check status
            let current = match self.db.get_task_by_id(&task.id) {
                Ok(Some(t)) => t,
                _ => continue,
            };
            if current.status != TaskStatus::Active {
                continue;
            }

            self.execute_task(&current).await;
        }
    }

    async fn execute_task(&self, task: &ScheduledTask) {
        info!(task_id = %task.id, workspace = %task.workspace, "Dispatching scheduled task");

        // Calculate next run
        let next_run = calculate_next_run(task);

        // Update next_run in DB before dispatching
        if let Err(e) = self
            .db
            .update_task_after_run(&task.id, next_run.as_deref(), "dispatched")
        {
            error!(task_id = %task.id, "Failed to update task after run: {}", e);
            return;
        }

        if let Err(e) = self.db.log_task_run(&TaskRunLog {
            task_id: task.id.clone(),
            run_at: Utc::now().to_rfc3339(),
            duration_ms: 0,
            status: RunStatus::Success,
            result: Some("dispatched to agent".into()),
            error: None,
        }) {
            error!(task_id = %task.id, "Failed to log task run: {}", e);
        }

        // Send to main loop for actual agent execution
        if let Err(e) = self.task_tx.send(task.clone()).await {
            error!(task_id = %task.id, "Failed to send task to main loop: {}", e);
        }
    }
}

/// Calculate initial next_run when a task is first created.
pub fn calculate_initial_next_run(task: &ScheduledTask) -> Option<String> {
    match task.schedule_type {
        ScheduleType::Cron => match task.schedule_value.parse::<Schedule>() {
            Ok(schedule) => schedule.upcoming(Utc).next().map(|t| t.to_rfc3339()),
            Err(e) => {
                warn!(task_id = %task.id, "Invalid cron expression: {}", e);
                None
            }
        },
        ScheduleType::Interval => {
            let ms: u64 = task.schedule_value.parse().unwrap_or(0);
            if ms > 0 {
                let next = Utc::now() + chrono::Duration::milliseconds(ms as i64);
                Some(next.to_rfc3339())
            } else {
                None
            }
        }
        ScheduleType::Once => {
            // schedule_value is an ISO timestamp — use it directly as next_run
            Some(task.schedule_value.clone())
        }
    }
}

/// Calculate next_run after a task has been executed.
/// Returns None for one-time tasks (marks them as completed).
pub fn calculate_next_run(task: &ScheduledTask) -> Option<String> {
    match task.schedule_type {
        ScheduleType::Cron => match task.schedule_value.parse::<Schedule>() {
            Ok(schedule) => schedule.upcoming(Utc).next().map(|t| t.to_rfc3339()),
            Err(e) => {
                warn!(task_id = %task.id, "Invalid cron expression: {}", e);
                None
            }
        },
        ScheduleType::Interval => {
            let ms: u64 = task.schedule_value.parse().unwrap_or(0);
            if ms > 0 {
                let next = Utc::now() + chrono::Duration::milliseconds(ms as i64);
                Some(next.to_rfc3339())
            } else {
                None
            }
        }
        ScheduleType::Once => None, // One-time tasks don't repeat
    }
}
