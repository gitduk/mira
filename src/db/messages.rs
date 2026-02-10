use crate::error::Result;
use crate::types::{ChatInfo, NewMessage};

use super::Database;

impl Database {
    pub fn store_chat_metadata(
        &self,
        chat_jid: &str,
        timestamp: &str,
        name: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn();
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
        let conn = self.conn();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO chats (jid, name, last_message_time) VALUES (?1, ?2, ?3)
             ON CONFLICT(jid) DO UPDATE SET name = excluded.name",
            rusqlite::params![chat_jid, name, now],
        )?;
        Ok(())
    }

    pub fn get_all_chats(&self) -> Result<Vec<ChatInfo>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT jid, name, last_message_time FROM chats ORDER BY last_message_time DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ChatInfo {
                jid: row.get(0)?,
                name: row.get(1)?,
                last_message_time: row.get(2)?,
            })
        })?;

        let mut chats = Vec::new();
        for row in rows {
            chats.push(row?);
        }
        Ok(chats)
    }

    pub fn get_last_group_sync(&self) -> Result<Option<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT last_message_time FROM chats WHERE jid = '__group_sync__'",
        )?;
        let result = stmt
            .query_row([], |row| row.get::<_, String>(0))
            .ok();
        Ok(result)
    }

    pub fn set_last_group_sync(&self) -> Result<()> {
        let conn = self.conn();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR REPLACE INTO chats (jid, name, last_message_time) VALUES ('__group_sync__', '__group_sync__', ?1)",
            rusqlite::params![now],
        )?;
        Ok(())
    }

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
        let conn = self.conn();
        conn.execute(
            "INSERT OR REPLACE INTO messages (id, chat_jid, sender, sender_name, content, timestamp, is_from_me) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![msg_id, chat_jid, sender, sender_name, content, timestamp, if is_from_me { 1 } else { 0 }],
        )?;
        Ok(())
    }

    pub fn get_new_messages(
        &self,
        jids: &[String],
        last_timestamp: &str,
        bot_prefix: &str,
    ) -> Result<(Vec<NewMessage>, String)> {
        if jids.is_empty() {
            return Ok((vec![], last_timestamp.to_string()));
        }

        let conn = self.conn();
        let placeholders: Vec<String> = (0..jids.len()).map(|i| format!("?{}", i + 2)).collect();
        let sql = format!(
            "SELECT id, chat_jid, sender, sender_name, content, timestamp
             FROM messages
             WHERE timestamp > ?1 AND chat_jid IN ({}) AND content NOT LIKE ?{}
             ORDER BY timestamp",
            placeholders.join(","),
            jids.len() + 2
        );

        let mut stmt = conn.prepare(&sql)?;

        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        params.push(Box::new(last_timestamp.to_string()));
        for jid in jids {
            params.push(Box::new(jid.clone()));
        }
        params.push(Box::new(format!("{}:%", bot_prefix)));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(NewMessage {
                id: row.get(0)?,
                chat_jid: row.get(1)?,
                sender: row.get(2)?,
                sender_name: row.get(3)?,
                content: row.get(4)?,
                timestamp: row.get(5)?,
            })
        })?;

        let mut messages = Vec::new();
        let mut new_timestamp = last_timestamp.to_string();
        for row in rows {
            let msg = row?;
            if msg.timestamp > new_timestamp {
                new_timestamp = msg.timestamp.clone();
            }
            messages.push(msg);
        }

        Ok((messages, new_timestamp))
    }

    pub fn get_messages_since(
        &self,
        chat_jid: &str,
        since_timestamp: &str,
        bot_prefix: &str,
    ) -> Result<Vec<NewMessage>> {
        let conn = self.conn();
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
                })
            },
        )?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    }
}
