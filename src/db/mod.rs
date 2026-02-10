pub mod groups;
pub mod messages;
pub mod state;
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
        let db_path = store_dir.join("messages.db");
        let conn = Connection::open(db_path)?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        let db = Database {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        db.run_migrations()?;

        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS chats (
                jid TEXT PRIMARY KEY,
                name TEXT,
                last_message_time TEXT
            );
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT,
                chat_jid TEXT,
                sender TEXT,
                sender_name TEXT,
                content TEXT,
                timestamp TEXT,
                is_from_me INTEGER,
                PRIMARY KEY (id, chat_jid),
                FOREIGN KEY (chat_jid) REFERENCES chats(jid)
            );
            CREATE INDEX IF NOT EXISTS idx_timestamp ON messages(timestamp);

            CREATE TABLE IF NOT EXISTS scheduled_tasks (
                id TEXT PRIMARY KEY,
                group_folder TEXT NOT NULL,
                chat_jid TEXT NOT NULL,
                prompt TEXT NOT NULL,
                schedule_type TEXT NOT NULL,
                schedule_value TEXT NOT NULL,
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

            CREATE TABLE IF NOT EXISTS router_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sessions (
                group_folder TEXT PRIMARY KEY,
                session_id TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS registered_groups (
                jid TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                folder TEXT NOT NULL UNIQUE,
                trigger_pattern TEXT NOT NULL,
                added_at TEXT NOT NULL,
                requires_trigger INTEGER DEFAULT 1
            );
            ",
        )?;
        Ok(())
    }

    fn run_migrations(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Add sender_name column if it doesn't exist
        let _ = conn.execute_batch("ALTER TABLE messages ADD COLUMN sender_name TEXT");

        // Add context_mode column if it doesn't exist
        let _ = conn.execute_batch(
            "ALTER TABLE scheduled_tasks ADD COLUMN context_mode TEXT DEFAULT 'isolated'",
        );

        // Add requires_trigger column if it doesn't exist
        let _ = conn.execute_batch(
            "ALTER TABLE registered_groups ADD COLUMN requires_trigger INTEGER DEFAULT 1",
        );

        Ok(())
    }

    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }
}
