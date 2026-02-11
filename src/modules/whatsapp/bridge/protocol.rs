use serde::{Deserialize, Serialize};

/// Commands sent from Rust to the Node.js bridge
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeCommand {
    SendMessage { jid: String, text: String },
    FetchGroups,
    Shutdown,
}

/// Events received from the Node.js bridge
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeEvent {
    Ready {
        #[serde(default)]
        user_jid: Option<String>,
        #[serde(default)]
        lid_jid: Option<String>,
    },
    Message {
        msg_id: String,
        chat_jid: String,
        sender: String,
        sender_name: String,
        content: String,
        timestamp: String,
        is_from_me: bool,
        #[serde(default)]
        chat_name: Option<String>,
    },
    ChatMetadata {
        chat_jid: String,
        timestamp: String,
        #[serde(default)]
        name: Option<String>,
    },
    Connection {
        status: String,
        #[serde(default)]
        reason: Option<i32>,
    },
    GroupsResult {
        groups: Vec<GroupMetadata>,
    },
    MessageSent {
        msg_id: String,
        jid: String,
    },
    Error {
        message: String,
    },
    Qr {
        code: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct GroupMetadata {
    pub jid: String,
    pub subject: String,
}
