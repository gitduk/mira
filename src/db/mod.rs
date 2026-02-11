pub mod tasks;

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::error::Result;

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn open(store_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(store_dir)?;
        let db_path = store_dir.join("mira.db");
        let conn = Connection::open(db_path)?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        let db = Database {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;

        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Migration: rename group_folder -> workspace (must run before CREATE TABLE IF NOT EXISTS)
        let has_old_column: bool = conn
            .prepare("SELECT group_folder FROM scheduled_tasks LIMIT 0")
            .is_ok();
        if has_old_column {
            conn.execute_batch(
                "ALTER TABLE scheduled_tasks RENAME COLUMN group_folder TO workspace;",
            )?;
        }

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS scheduled_tasks (
                id TEXT PRIMARY KEY,
                workspace TEXT NOT NULL,
                chat_jid TEXT NOT NULL,
                prompt TEXT NOT NULL,
                schedule_type TEXT NOT NULL,
                schedule_value TEXT NOT NULL,
                context_mode TEXT DEFAULT 'isolated',
                module_id TEXT NOT NULL,
                next_run TEXT,
                last_run TEXT,
                last_result TEXT,
                status TEXT DEFAULT 'active',
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_next_run ON scheduled_tasks(next_run);
            CREATE INDEX IF NOT EXISTS idx_status ON scheduled_tasks(status);

            CREATE TABLE IF NOT EXISTS task_run_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL,
                run_at TEXT NOT NULL,
                duration_ms INTEGER NOT NULL,
                status TEXT NOT NULL,
                result TEXT,
                error TEXT,
                FOREIGN KEY (task_id) REFERENCES scheduled_tasks(id)
            );
            CREATE INDEX IF NOT EXISTS idx_task_run_logs ON task_run_logs(task_id, run_at);

            ",
        )?;

        Ok(())
    }

    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }
}
