use std::sync::Arc;

use chrono::Utc;
use cron::Schedule;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::db::Database;
use crate::types::{RunStatus, ScheduleType, ScheduledTask, TaskRunLog, TaskStatus};

pub struct Scheduler {
    db: Arc<Database>,
    config: Arc<Config>,
    running: bool,
}

impl Scheduler {
    pub fn new(db: Arc<Database>, config: Arc<Config>) -> Self {
        Scheduler {
            db,
            config,
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

            // Run the task (in the future, this would go through the ModuleQueue)
            self.execute_task(&current).await;
        }
    }

    async fn execute_task(&self, task: &ScheduledTask) {
        let start = std::time::Instant::now();
        info!(task_id = %task.id, group = %task.group_folder, "Running scheduled task");

        // For now, we just log that the task should run
        // The actual agent execution will be handled by the main loop
        let duration_ms = start.elapsed().as_millis() as i64;

        // Calculate next run
        let next_run = calculate_next_run(task);

        let result_summary = "Task execution delegated to agent loop";
        if let Err(e) = self
            .db
            .update_task_after_run(&task.id, next_run.as_deref(), result_summary)
        {
            error!(task_id = %task.id, "Failed to update task after run: {}", e);
        }

        if let Err(e) = self.db.log_task_run(&TaskRunLog {
            task_id: task.id.clone(),
            run_at: Utc::now().to_rfc3339(),
            duration_ms,
            status: RunStatus::Success,
            result: Some(result_summary.to_string()),
            error: None,
        }) {
            error!(task_id = %task.id, "Failed to log task run: {}", e);
        }
    }
}

pub fn calculate_next_run(task: &ScheduledTask) -> Option<String> {
    match task.schedule_type {
        ScheduleType::Cron => {
            // Parse cron expression and get next occurrence
            match task.schedule_value.parse::<Schedule>() {
                Ok(schedule) => schedule.upcoming(Utc).next().map(|t| t.to_rfc3339()),
                Err(e) => {
                    warn!(task_id = %task.id, "Invalid cron expression: {}", e);
                    None
                }
            }
        }
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
