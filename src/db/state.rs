use std::collections::HashMap;

use crate::error::Result;

use super::Database;

impl Database {
    pub fn get_router_state(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT value FROM router_state WHERE key = ?1")?;
        let result = stmt
            .query_row(rusqlite::params![key], |row| row.get::<_, String>(0))
            .ok();
        Ok(result)
    }

    pub fn set_router_state(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR REPLACE INTO router_state (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    pub fn get_session(&self, group_folder: &str) -> Result<Option<String>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT session_id FROM sessions WHERE group_folder = ?1")?;
        let result = stmt
            .query_row(rusqlite::params![group_folder], |row| {
                row.get::<_, String>(0)
            })
            .ok();
        Ok(result)
    }

    pub fn set_session(&self, group_folder: &str, session_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR REPLACE INTO sessions (group_folder, session_id) VALUES (?1, ?2)",
            rusqlite::params![group_folder, session_id],
        )?;
        Ok(())
    }

    pub fn get_all_sessions(&self) -> Result<HashMap<String, String>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT group_folder, session_id FROM sessions")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut sessions = HashMap::new();
        for row in rows {
            let (folder, session_id) = row?;
            sessions.insert(folder, session_id);
        }
        Ok(sessions)
    }
}
