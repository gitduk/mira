use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::error::Result;
use crate::types::NewMessage;

use super::types::RegisteredGroup;

pub struct WhatsAppStore {
    conn: Mutex<Connection>,
}

impl WhatsAppStore {
    pub fn open(store_dir: &Path, _main_db: &crate::db::Database) -> Result<Self> {
        let module_dir = store_dir.join("whatsapp");
        std::fs::create_dir_all(&module_dir)?;
        let db_path = module_dir.join("module.db");
        let conn = Connection::open(db_path)?;

        let store = WhatsAppStore {
            conn: Mutex::new(conn),
        };
        store.init_schema()?;

        Ok(store)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        // Migration: rename group_folder -> workspace in sessions table (before CREATE TABLE)
        let has_old_column: bool = conn
            .prepare("SELECT group_folder FROM sessions LIMIT 0")
            .is_ok();
        if has_old_column {
            conn.execute_batch("ALTER TABLE sessions RENAME COLUMN group_folder TO workspace;")?;
        }

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
                PRIMARY KEY (id, chat_jid)
            );
            CREATE INDEX IF NOT EXISTS idx_timestamp ON messages(timestamp);

            CREATE TABLE IF NOT EXISTS router_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sessions (
                workspace TEXT PRIMARY KEY,
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

    pub fn store_chat_metadata(
        &self,
        chat_jid: &str,
        timestamp: &str,
        name: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        if let Some(name) = name {
            conn.execute(
                "INSERT INTO chats (jid, name, last_message_time) VALUES (?1, ?2, ?3)
                 ON CONFLICT(jid) DO UPDATE SET
                   name = excluded.name,
                   last_message_time = MAX(last_message_time, excluded.last_message_time)",
                rusqlite::params![chat_jid, name, timestamp],
            )?;
        } else {
            conn.execute(
                "INSERT INTO chats (jid, name, last_message_time) VALUES (?1, ?1, ?2)
                 ON CONFLICT(jid) DO UPDATE SET
                   last_message_time = MAX(last_message_time, excluded.last_message_time)",
                rusqlite::params![chat_jid, timestamp],
            )?;
        }
        Ok(())
    }

    pub fn update_chat_name(&self, chat_jid: &str, name: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO chats (jid, name, last_message_time) VALUES (?1, ?2, ?3)
             ON CONFLICT(jid) DO UPDATE SET name = excluded.name",
            rusqlite::params![chat_jid, name, now],
        )?;
        Ok(())
    }

    pub fn set_last_group_sync(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR REPLACE INTO chats (jid, name, last_message_time) VALUES ('__group_sync__', '__group_sync__', ?1)",
            rusqlite::params![now],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn store_message(
        &self,
        msg_id: &str,
        chat_jid: &str,
        sender: &str,
        sender_name: &str,
        content: &str,
        timestamp: &str,
        is_from_me: bool,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO messages (id, chat_jid, sender, sender_name, content, timestamp, is_from_me)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                msg_id,
                chat_jid,
                sender,
                sender_name,
                content,
                timestamp,
                if is_from_me { 1 } else { 0 }
            ],
        )?;
        Ok(())
    }

    pub fn get_messages_since(
        &self,
        chat_jid: &str,
        since_timestamp: &str,
        bot_prefix: &str,
    ) -> Result<Vec<NewMessage>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, chat_jid, sender, sender_name, content, timestamp
             FROM messages
             WHERE chat_jid = ?1 AND timestamp > ?2 AND content NOT LIKE ?3
             ORDER BY timestamp",
        )?;

        let rows = stmt.query_map(
            rusqlite::params![chat_jid, since_timestamp, format!("{}:%", bot_prefix)],
            |row| {
                Ok(NewMessage {
                    id: row.get(0)?,
                    chat_jid: row.get(1)?,
                    sender: row.get(2)?,
                    sender_name: row.get(3)?,
                    content: row.get(4)?,
                    timestamp: row.get(5)?,
                    module_id: None,
                })
            },
        )?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    }

    pub fn get_router_state(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM router_state WHERE key = ?1")?;
        let result = stmt
            .query_row(rusqlite::params![key], |row| row.get::<_, String>(0))
            .ok();
        Ok(result)
    }

    pub fn set_router_state(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO router_state (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    pub fn get_session(&self, workspace: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT session_id FROM sessions WHERE workspace = ?1")?;
        let result = stmt
            .query_row(rusqlite::params![workspace], |row| row.get::<_, String>(0))
            .ok();
        Ok(result)
    }

    pub fn set_session(&self, workspace: &str, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO sessions (workspace, session_id) VALUES (?1, ?2)",
            rusqlite::params![workspace, session_id],
        )?;
        Ok(())
    }

    pub fn get_registered_groups(&self) -> Result<HashMap<String, RegisteredGroup>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT jid, name, folder, trigger_pattern, added_at, requires_trigger
             FROM registered_groups",
        )?;
        let rows = stmt.query_map([], |row| {
            let jid: String = row.get(0)?;
            let requires_trigger: Option<i32> = row.get(5)?;
            Ok((
                jid.clone(),
                RegisteredGroup {
                    jid,
                    name: row.get(1)?,
                    folder: row.get(2)?,
                    trigger: row.get(3)?,
                    added_at: row.get(4)?,
                    requires_trigger: requires_trigger.map(|v| v == 1),
                },
            ))
        })?;

        let mut groups = HashMap::new();
        for row in rows {
            let (jid, group) = row?;
            groups.insert(jid, group);
        }
        Ok(groups)
    }

    /// Get private chat JIDs (ending in @s.whatsapp.net) that have messages,
    /// excluding any JIDs already in registered_groups.
    /// Returns Vec<(chat_jid, last_agent_ts)>.
    pub fn get_active_private_chats(&self, module_id: &str) -> Result<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT m.chat_jid
             FROM messages m
             LEFT JOIN registered_groups rg ON m.chat_jid = rg.jid
             WHERE m.chat_jid NOT LIKE '%@g.us'
               AND m.chat_jid NOT LIKE '%@broadcast'
               AND rg.jid IS NULL",
        )?;

        let jids: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();

        let mut result = Vec::new();
        for jid in jids {
            let key = format!("last_agent_ts:{}:{}", module_id, jid);
            let last_ts = self.get_router_state_inner(&conn, &key).unwrap_or_default();

            // Check if there are messages newer than last_agent_ts
            let has_new: bool = conn
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM messages
                        WHERE chat_jid = ?1 AND timestamp > ?2 AND is_from_me = 0
                    )",
                    rusqlite::params![jid, last_ts],
                    |row| row.get(0),
                )
                .unwrap_or(false);

            if has_new {
                result.push((jid, last_ts));
            }
        }

        Ok(result)
    }

    pub fn get_chat_name(&self, chat_jid: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT name FROM chats WHERE jid = ?1")?;
        let result = stmt
            .query_row(rusqlite::params![chat_jid], |row| row.get::<_, String>(0))
            .ok();
        Ok(result)
    }

    fn get_router_state_inner(&self, conn: &Connection, key: &str) -> Option<String> {
        let mut stmt = conn
            .prepare("SELECT value FROM router_state WHERE key = ?1")
            .ok()?;
        stmt.query_row(rusqlite::params![key], |row| row.get::<_, String>(0))
            .ok()
    }

    pub fn set_registered_group(&self, jid: &str, group: &RegisteredGroup) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let requires_trigger = match group.requires_trigger {
            Some(true) => 1,
            Some(false) => 0,
            None => 1,
        };
        conn.execute(
            "INSERT OR REPLACE INTO registered_groups (jid, name, folder, trigger_pattern, added_at, requires_trigger)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                jid,
                group.name,
                group.folder,
                group.trigger,
                group.added_at,
                requires_trigger
            ],
        )?;
        Ok(())
    }
}
