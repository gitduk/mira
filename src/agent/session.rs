use std::path::{Path, PathBuf};

use tracing::{debug, error, info};
use uuid::Uuid;

use super::api::Message;
use crate::config::Config;

const SESSIONS_DIR: &str = ".sessions";

#[derive(Debug)]
pub struct Session {
    pub id: String,
    pub messages: Vec<Message>,
    session_dir: PathBuf,
    persist: bool,
}

impl Session {
    pub fn load_or_create(workspace_dir: &Path, session_id: Option<&str>, persist: bool) -> Self {
        let session_dir = workspace_dir.join(SESSIONS_DIR);
        if persist {
            let _ = std::fs::create_dir_all(&session_dir);
        }

        if persist {
            if let Some(sid) = session_id {
                let session_file = session_dir.join(format!("{}.jsonl", sid));
                if session_file.exists() {
                    match Self::load_from_file(&session_file, sid) {
                        Ok(session) => {
                            info!(
                                session_id = sid,
                                messages = session.messages.len(),
                                "Resumed session"
                            );
                            return session;
                        }
                        Err(e) => {
                            error!("Failed to load session {}: {}", sid, e);
                        }
                    }
                }
            }
        }

        let id = Uuid::new_v4().to_string();
        debug!(session_id = %id, "Created new session");
        Session {
            id,
            messages: Vec::new(),
            session_dir,
            persist,
        }
    }

    fn load_from_file(path: &Path, session_id: &str) -> crate::error::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut messages = Vec::new();

        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(msg) = serde_json::from_str::<Message>(line) {
                messages.push(msg);
            }
        }

        Ok(Session {
            id: session_id.to_string(),
            messages,
            session_dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            persist: true,
        })
    }

    pub fn save(&self, workspace_dir: &Path) -> crate::error::Result<()> {
        if !self.persist {
            return Ok(());
        }
        let session_dir = workspace_dir.join(SESSIONS_DIR);
        std::fs::create_dir_all(&session_dir)?;

        let session_file = session_dir.join(format!("{}.jsonl", self.id));
        let mut content = String::new();

        for msg in &self.messages {
            let line = serde_json::to_string(msg)?;
            content.push_str(&line);
            content.push('\n');
        }

        std::fs::write(&session_file, content)?;
        debug!(session_id = %self.id, messages = self.messages.len(), "Session saved");
        Ok(())
    }

    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);
    }

    pub fn estimated_tokens(&self) -> usize {
        // Rough estimate: ~4 chars per token
        let total_chars: usize = self
            .messages
            .iter()
            .map(|m| {
                m.content
                    .iter()
                    .map(|c| match c {
                        super::api::ContentBlock::Text { text } => text.len(),
                        super::api::ContentBlock::ToolUse { input, .. } => {
                            serde_json::to_string(input).map_or(0, |s| s.len())
                        }
                        super::api::ContentBlock::ToolResult { content, .. } => content.len(),
                    })
                    .sum::<usize>()
            })
            .sum();
        total_chars / 4
    }
}

pub fn build_system_prompt(
    config: &Config,
    workspace_dir: &Path,
    global_claude_md: Option<&str>,
) -> String {
    let now_utc = chrono::Utc::now();
    let mut prompt = format!(
        "You are {}, a helpful AI assistant. You have access to various tools to help accomplish tasks.\n\n\
         Current time (UTC): {}\nTimezone: {}\n\n",
        config.assistant_name,
        now_utc.to_rfc3339(),
        config.timezone,
    );

    // Load group CLAUDE.md
    let claude_md_path = workspace_dir.join("CLAUDE.md");
    if claude_md_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&claude_md_path) {
            prompt.push_str("## Group Instructions\n\n");
            prompt.push_str(&content);
            prompt.push_str("\n\n");
        }
    }

    // Append global CLAUDE.md
    if let Some(global) = global_claude_md {
        prompt.push_str("## Global Instructions\n\n");
        prompt.push_str(global);
        prompt.push_str("\n\n");
    }

    prompt
}
