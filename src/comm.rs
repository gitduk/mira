use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::dispatch::ModuleWorkItem;
use crate::error::Result;
use crate::module::ModuleStatus;

/// Module-agnostic channel address. Opaque to Mira core.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ChannelAddr {
    pub module_id: String,
    pub channel_id: String,
}

impl ChannelAddr {
    pub fn new(module_id: impl Into<String>, channel_id: impl Into<String>) -> Self {
        ChannelAddr {
            module_id: module_id.into(),
            channel_id: channel_id.into(),
        }
    }

    /// Unique string key for use in HashMaps (e.g. ModuleQueue).
    pub fn as_key(&self) -> String {
        format!("{}:{}", self.module_id, self.channel_id)
    }
}

impl std::fmt::Display for ChannelAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.module_id, self.channel_id)
    }
}

/// A message flowing from a module into Mira core.
#[derive(Debug, Clone)]
pub struct MiraMessage {
    pub addr: ChannelAddr,
    pub msg_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub content: String,
    pub timestamp: String,
    pub is_from_self: bool,
    pub channel_name: Option<String>,
}

/// Events emitted by a module toward Mira core.
#[derive(Debug)]
pub enum ModuleEvent {
    Message(MiraMessage),
    StatusChange {
        module_id: String,
        status: ModuleStatus,
    },
    Log {
        module_id: String,
        message: String,
    },
    Error {
        module_id: String,
        message: String,
    },
}

/// Commands sent from Mira core to a module.
#[derive(Debug)]
pub enum ModuleCommand {
    SendMessage {
        channel_id: String,
        text: String,
    },
    SendPresence {
        channel_id: String,
        presence: String,
    },
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct ModuleToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone)]
pub struct ModuleToolCall {
    pub name: String,
    pub input: Value,
}

#[derive(Debug, Clone)]
pub struct ModuleToolResult {
    pub content: String,
    pub is_error: bool,
}

/// Trait that every communication module must implement.
#[async_trait]
pub trait CommunicationModule: Send + Sync {
    /// Unique module identifier (e.g. "whatsapp", "telegram").
    fn id(&self) -> &str;

    /// Human-readable name for dashboard display.
    fn display_name(&self) -> &str;

    /// Start the module. Returns a receiver for events emitted by this module.
    async fn start(&mut self) -> Result<mpsc::Receiver<ModuleEvent>>;

    /// Send a command to this module.
    async fn send_command(&self, cmd: ModuleCommand) -> Result<()>;

    /// List module-specific tools.
    fn tools(&self) -> Vec<ModuleToolDefinition> {
        Vec::new()
    }

    /// Execute a module-specific tool.
    async fn call_tool(&mut self, _call: ModuleToolCall) -> Result<ModuleToolResult> {
        Ok(ModuleToolResult {
            content: "Tool not implemented".into(),
            is_error: true,
        })
    }

    /// Drain pending work items for Mira to execute.
    async fn drain_work_items(&self) -> Result<Vec<ModuleWorkItem>> {
        Ok(Vec::new())
    }

    /// Get session id for a group folder (module-scoped).
    async fn get_session_id(&self, _group_folder: &str) -> Result<Option<String>> {
        Ok(None)
    }

    /// Persist session id for a group folder (module-scoped).
    async fn set_session_id(&self, _group_folder: &str, _session_id: &str) -> Result<()> {
        Ok(())
    }

    /// Whether this module should persist agent sessions to disk.
    fn persist_sessions(&self) -> bool {
        true
    }

    /// Gracefully shut down the module.
    async fn shutdown(&mut self);
}
