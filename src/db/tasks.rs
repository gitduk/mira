use crate::error::Result;
use crate::types::{ContextMode, ScheduleType, ScheduledTask, TaskRunLog, TaskStatus};

use super::Database;

impl Database {
    pub fn create_task(&self, task: &ScheduledTask) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO scheduled_tasks (id, group_folder, chat_jid, prompt, schedule_type, schedule_value, context_mode, next_run, status, created_at, module_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                task.id,
                task.group_folder,
                task.chat_jid,
                task.prompt,
                task.schedule_type.as_str(),
                task.schedule_value,
                task.context_mode.as_str(),
                task.next_run,
                task.status.as_str(),
                task.created_at,
                task.module_id,
            ],
        )?;
        Ok(())
    }

    pub fn get_task_by_id(&self, id: &str) -> Result<Option<ScheduledTask>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, group_folder, chat_jid, prompt, schedule_type, schedule_value, next_run, last_run, last_result, status, created_at, context_mode, module_id
             FROM scheduled_tasks WHERE id = ?1",
        )?;
        let result = stmt
            .query_row(rusqlite::params![id], |row| Ok(row_to_task(row)))
            .ok();
        Ok(result)
    }

    pub fn get_tasks_for_group(&self, group_folder: &str) -> Result<Vec<ScheduledTask>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, group_folder, chat_jid, prompt, schedule_type, schedule_value, next_run, last_run, last_result, status, created_at, context_mode, module_id
             FROM scheduled_tasks WHERE group_folder = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![group_folder], |row| Ok(row_to_task(row)))?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row?);
        }
        Ok(tasks)
    }

    pub fn get_all_tasks(&self) -> Result<Vec<ScheduledTask>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, group_folder, chat_jid, prompt, schedule_type, schedule_value, next_run, last_run, last_result, status, created_at, context_mode, module_id
             FROM scheduled_tasks ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| Ok(row_to_task(row)))?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row?);
        }
        Ok(tasks)
    }

    pub fn update_task_status(&self, id: &str, status: &TaskStatus) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "UPDATE scheduled_tasks SET status = ?1 WHERE id = ?2",
            rusqlite::params![status.as_str(), id],
        )?;
        Ok(())
    }

    pub fn update_task_fields(
        &self,
        id: &str,
        prompt: Option<&str>,
        schedule_type: Option<&str>,
        schedule_value: Option<&str>,
        next_run: Option<Option<&str>>,
        status: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn();
        let mut fields = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(p) = prompt {
            fields.push("prompt = ?");
            values.push(Box::new(p.to_string()));
        }
        if let Some(st) = schedule_type {
            fields.push("schedule_type = ?");
            values.push(Box::new(st.to_string()));
        }
        if let Some(sv) = schedule_value {
            fields.push("schedule_value = ?");
            values.push(Box::new(sv.to_string()));
        }
        if let Some(nr) = next_run {
            fields.push("next_run = ?");
            values.push(Box::new(nr.map(|s| s.to_string())));
        }
        if let Some(s) = status {
            fields.push("status = ?");
            values.push(Box::new(s.to_string()));
        }

        if fields.is_empty() {
            return Ok(());
        }

        values.push(Box::new(id.to_string()));
        let sql = format!(
            "UPDATE scheduled_tasks SET {} WHERE id = ?",
            fields.join(", ")
        );

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            values.iter().map(|p| p.as_ref()).collect();
        conn.execute(&sql, param_refs.as_slice())?;
        Ok(())
    }

    pub fn delete_task(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM task_run_logs WHERE task_id = ?1",
            rusqlite::params![id],
        )?;
        conn.execute(
            "DELETE FROM scheduled_tasks WHERE id = ?1",
            rusqlite::params![id],
        )?;
        Ok(())
    }

    pub fn get_due_tasks(&self) -> Result<Vec<ScheduledTask>> {
        let conn = self.conn();
        let now = chrono::Utc::now().to_rfc3339();
        let mut stmt = conn.prepare(
            "SELECT id, group_folder, chat_jid, prompt, schedule_type, schedule_value, next_run, last_run, last_result, status, created_at, context_mode, module_id
             FROM scheduled_tasks
             WHERE status = 'active' AND next_run IS NOT NULL AND next_run <= ?1
             ORDER BY next_run",
        )?;
        let rows = stmt.query_map(rusqlite::params![now], |row| Ok(row_to_task(row)))?;

        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row?);
        }
        Ok(tasks)
    }

    pub fn update_task_after_run(
        &self,
        id: &str,
        next_run: Option<&str>,
        last_result: &str,
    ) -> Result<()> {
        let conn = self.conn();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE scheduled_tasks
             SET next_run = ?1, last_run = ?2, last_result = ?3,
                 status = CASE WHEN ?1 IS NULL THEN 'completed' ELSE status END
             WHERE id = ?4",
            rusqlite::params![next_run, now, last_result, id],
        )?;
        Ok(())
    }

    pub fn log_task_run(&self, log: &TaskRunLog) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO task_run_logs (task_id, run_at, duration_ms, status, result, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                log.task_id,
                log.run_at,
                log.duration_ms,
                log.status.as_str(),
                log.result,
                log.error,
            ],
        )?;
        Ok(())
    }
}

fn row_to_task(row: &rusqlite::Row<'_>) -> ScheduledTask {
    ScheduledTask {
        id: row.get(0).unwrap_or_default(),
        group_folder: row.get(1).unwrap_or_default(),
        chat_jid: row.get(2).unwrap_or_default(),
        prompt: row.get(3).unwrap_or_default(),
        schedule_type: ScheduleType::from_str(&row.get::<_, String>(4).unwrap_or_default())
            .unwrap_or(ScheduleType::Once),
        schedule_value: row.get(5).unwrap_or_default(),
        next_run: row.get(6).ok(),
        last_run: row.get(7).ok(),
        last_result: row.get(8).ok(),
        status: TaskStatus::from_str(&row.get::<_, String>(9).unwrap_or_default()),
        created_at: row.get(10).unwrap_or_default(),
        context_mode: ContextMode::from_str(
            &row.get::<_, String>(11)
                .unwrap_or_else(|_| "isolated".into()),
        ),
        module_id: row
            .get::<_, String>(12)
            .unwrap_or_else(|_| "whatsapp".into()),
    }
}
