use std::collections::HashMap;

use crate::error::Result;
use crate::types::RegisteredGroup;

use super::Database;

impl Database {
    pub fn get_registered_group(&self, jid: &str) -> Result<Option<RegisteredGroup>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT name, folder, trigger_pattern, added_at, requires_trigger FROM registered_groups WHERE jid = ?1")?;
        let result = stmt
            .query_row(rusqlite::params![jid], |row| {
                let requires_trigger: Option<i32> = row.get(4)?;
                Ok(RegisteredGroup {
                    name: row.get(0)?,
                    folder: row.get(1)?,
                    trigger: row.get(2)?,
                    added_at: row.get(3)?,
                    requires_trigger: requires_trigger.map(|v| v == 1),
                })
            })
            .ok();
        Ok(result)
    }

    pub fn set_registered_group(&self, jid: &str, group: &RegisteredGroup) -> Result<()> {
        let conn = self.conn();
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
                requires_trigger,
            ],
        )?;
        Ok(())
    }

    pub fn get_all_registered_groups(&self) -> Result<HashMap<String, RegisteredGroup>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT jid, name, folder, trigger_pattern, added_at, requires_trigger FROM registered_groups",
        )?;
        let rows = stmt.query_map([], |row| {
            let jid: String = row.get(0)?;
            let requires_trigger: Option<i32> = row.get(5)?;
            Ok((
                jid,
                RegisteredGroup {
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
}
