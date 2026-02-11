use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use super::bridge::protocol::{BridgeCommand, BridgeEvent};
use super::bridge::{BridgeCommandSender, BridgeManager};
use crate::comm::{
    ChannelAddr, CommunicationModule, MiraMessage, ModuleCommand, ModuleEvent, ModuleToolCall,
    ModuleToolDefinition, ModuleToolResult,
};
use crate::config::Config;
use crate::dispatch::ModuleWorkItem;
use crate::error::Result;
use crate::module::ModuleStatus;
use crate::modules::whatsapp::store::WhatsAppStore;
use crate::modules::whatsapp::types::RegisteredGroup;
use crate::prompt::xml_escape;
use serde_json::json;

const MODULE_ID: &str = "whatsapp";

pub struct WhatsAppModule {
    bridge_dir: PathBuf,
    store_dir: PathBuf,
    config: Arc<Config>,
    store: Arc<WhatsAppStore>,
    bridge_cmd_sender: Option<BridgeCommandSender>,
}

impl WhatsAppModule {
    pub fn new(
        bridge_dir: PathBuf,
        store_dir: PathBuf,
        db: Arc<crate::db::Database>,
        config: Arc<Config>,
    ) -> Self {
        let store =
            WhatsAppStore::open(&store_dir, &db).expect("Failed to open WhatsApp module db");
        WhatsAppModule {
            bridge_dir,
            store_dir,
            config,
            store: Arc::new(store),
            bridge_cmd_sender: None,
        }
    }
}

#[async_trait]
impl CommunicationModule for WhatsAppModule {
    fn id(&self) -> &str {
        MODULE_ID
    }

    fn display_name(&self) -> &str {
        "WhatsApp"
    }

    async fn start(&mut self) -> Result<mpsc::Receiver<ModuleEvent>> {
        let (module_event_tx, module_event_rx) = mpsc::channel::<ModuleEvent>(200);
        let (bridge_event_tx, mut bridge_event_rx) = mpsc::channel::<BridgeEvent>(100);

        // Create bridge manager and grab its shared command sender before spawning
        let bridge_dir = self.bridge_dir.clone();
        let store_dir = self.store_dir.join("whatsapp");
        let mut manager = BridgeManager::new(bridge_dir, store_dir, bridge_event_tx);
        let cmd_sender = manager.command_sender();
        self.bridge_cmd_sender = Some(cmd_sender);

        // Run bridge manager in background
        tokio::spawn(async move {
            if let Err(e) = manager.run().await {
                error!("Bridge manager error: {}", e);
            }
        });

        // Translate BridgeEvent -> ModuleEvent
        let store = self.store.clone();
        tokio::spawn(async move {
            while let Some(event) = bridge_event_rx.recv().await {
                let module_event = match event {
                    BridgeEvent::Ready { user_jid, lid_jid } => {
                        info!(?user_jid, ?lid_jid, "WhatsApp bridge ready");
                        Some(ModuleEvent::StatusChange {
                            module_id: MODULE_ID.into(),
                            status: ModuleStatus::Connected,
                        })
                    }
                    BridgeEvent::Message {
                        msg_id,
                        chat_jid,
                        sender,
                        sender_name,
                        content,
                        timestamp,
                        is_from_me,
                        chat_name,
                    } => {
                        let _ =
                            store.store_chat_metadata(&chat_jid, &timestamp, chat_name.as_deref());
                        let _ = store.store_message(
                            &msg_id,
                            &chat_jid,
                            &sender,
                            &sender_name,
                            &content,
                            &timestamp,
                            is_from_me,
                        );
                        Some(ModuleEvent::Message(MiraMessage {
                            addr: ChannelAddr::new(MODULE_ID, &chat_jid),
                            msg_id,
                            sender_id: sender,
                            sender_name,
                            content,
                            timestamp,
                            is_from_self: is_from_me,
                            channel_name: chat_name,
                        }))
                    }
                    BridgeEvent::ChatMetadata {
                        chat_jid,
                        timestamp,
                        name,
                    } => {
                        let _ = store.store_chat_metadata(&chat_jid, &timestamp, name.as_deref());
                        None
                    }
                    BridgeEvent::Connection { status, reason } => {
                        let module_status = match status.as_str() {
                            "open" => ModuleStatus::Connected,
                            "close" => {
                                if reason.map_or(true, |r| r != 401 && r != 440) {
                                    ModuleStatus::Reconnecting
                                } else {
                                    ModuleStatus::Disconnected
                                }
                            }
                            _ => ModuleStatus::Connecting,
                        };
                        Some(ModuleEvent::StatusChange {
                            module_id: MODULE_ID.into(),
                            status: module_status,
                        })
                    }
                    BridgeEvent::GroupsResult { groups } => {
                        let mut lines = Vec::new();
                        for g in &groups {
                            let _ = store.update_chat_name(&g.jid, &g.subject);
                            lines.push(format!("- {} ({})", g.subject, g.jid));
                        }
                        let _ = store.set_last_group_sync();
                        let message = if lines.is_empty() {
                            "No groups found.".to_string()
                        } else {
                            format!("Groups:\n{}", lines.join("\n"))
                        };
                        Some(ModuleEvent::Log {
                            module_id: MODULE_ID.into(),
                            message,
                        })
                    }
                    BridgeEvent::Error { message } => Some(ModuleEvent::Error {
                        module_id: MODULE_ID.into(),
                        message,
                    }),
                    BridgeEvent::Qr { .. } => Some(ModuleEvent::Error {
                        module_id: MODULE_ID.into(),
                        message: "WhatsApp QR required - run setup first".into(),
                    }),
                    BridgeEvent::MessageSent { .. } => None,
                };

                if let Some(evt) = module_event {
                    if module_event_tx.send(evt).await.is_err() {
                        break;
                    }
                }
            }
        });

        Ok(module_event_rx)
    }

    async fn send_command(&self, cmd: ModuleCommand) -> Result<()> {
        let Some(sender) = &self.bridge_cmd_sender else {
            warn!("WhatsApp module not started, cannot send command");
            return Ok(());
        };

        match cmd {
            ModuleCommand::SendMessage { channel_id, text } => {
                sender
                    .send(BridgeCommand::SendMessage {
                        jid: channel_id,
                        text,
                    })
                    .await?;
            }
            ModuleCommand::SendPresence {
                channel_id,
                presence,
            } => {
                sender
                    .send(BridgeCommand::SendPresence {
                        jid: channel_id,
                        presence,
                    })
                    .await?;
            }
            ModuleCommand::Shutdown => {
                let _ = sender.send(BridgeCommand::Shutdown).await;
            }
        }
        Ok(())
    }

    fn tools(&self) -> Vec<ModuleToolDefinition> {
        vec![
            ModuleToolDefinition {
                name: "fetch_groups".into(),
                description: "Fetch all WhatsApp groups.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            ModuleToolDefinition {
                name: "register_group".into(),
                description: "Register a WhatsApp group for Mira.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "jid": { "type": "string" },
                        "name": { "type": "string" },
                        "folder": { "type": "string" },
                        "trigger": { "type": "string" }
                    },
                    "required": ["jid", "name", "folder", "trigger"]
                }),
            },
            ModuleToolDefinition {
                name: "authorize_schedule_task".into(),
                description: "Authorize a schedule_task request for WhatsApp.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "source_group": { "type": "string" },
                        "target_jid": { "type": "string" }
                    },
                    "required": ["source_group", "target_jid"]
                }),
            },
        ]
    }

    async fn call_tool(&mut self, call: ModuleToolCall) -> Result<ModuleToolResult> {
        match call.name.as_str() {
            "fetch_groups" => {
                if let Some(sender) = &self.bridge_cmd_sender {
                    let _ = sender.send(BridgeCommand::FetchGroups).await;
                    Ok(ModuleToolResult {
                        content: "Fetch groups requested.".into(),
                        is_error: false,
                    })
                } else {
                    Ok(ModuleToolResult {
                        content: "WhatsApp module not started.".into(),
                        is_error: true,
                    })
                }
            }
            "register_group" => {
                let jid = call.input["jid"].as_str().unwrap_or_default();
                let name = call.input["name"].as_str().unwrap_or_default();
                let folder = call.input["folder"].as_str().unwrap_or_default();
                let trigger = call.input["trigger"].as_str().unwrap_or_default();

                if jid.is_empty() || name.is_empty() || folder.is_empty() || trigger.is_empty() {
                    return Ok(ModuleToolResult {
                        content: "jid, name, folder, and trigger are required".into(),
                        is_error: true,
                    });
                }

                let group = RegisteredGroup {
                    jid: jid.to_string(),
                    name: name.to_string(),
                    folder: folder.to_string(),
                    trigger: trigger.to_string(),
                    added_at: chrono::Utc::now().to_rfc3339(),
                    requires_trigger: None,
                };
                let _ = self.store.set_registered_group(jid, &group);

                let group_dir = self.config.groups_dir.join(folder);
                let _ = std::fs::create_dir_all(group_dir.join("logs"));

                Ok(ModuleToolResult {
                    content: format!("Group \"{}\" registered.", name),
                    is_error: false,
                })
            }
            "authorize_schedule_task" => {
                let source_group = call.input["source_group"].as_str().unwrap_or_default();
                let target_jid = call.input["target_jid"].as_str().unwrap_or_default();

                if source_group.is_empty() || target_jid.is_empty() {
                    return Ok(ModuleToolResult {
                        content: "source_group and target_jid are required".into(),
                        is_error: true,
                    });
                }

                if self.config.is_main_group(source_group) {
                    return Ok(ModuleToolResult {
                        content: "authorized".into(),
                        is_error: false,
                    });
                }

                let groups = self.store.get_registered_groups().unwrap_or_default();
                if let Some(group) = groups.get(target_jid) {
                    if group.folder == source_group {
                        return Ok(ModuleToolResult {
                            content: "authorized".into(),
                            is_error: false,
                        });
                    }
                }

                Ok(ModuleToolResult {
                    content: "unauthorized".into(),
                    is_error: true,
                })
            }
            _ => Ok(ModuleToolResult {
                content: format!("Unknown tool: {}", call.name),
                is_error: true,
            }),
        }
    }

    async fn drain_work_items(&self) -> Result<Vec<ModuleWorkItem>> {
        let mut work_items = Vec::new();

        // 1. Process registered groups
        let groups = self.store.get_registered_groups().unwrap_or_default();
        for (jid, group) in &groups {
            let key = format!("last_agent_ts:{}:{}", MODULE_ID, jid);
            let last_agent_ts = self
                .store
                .get_router_state(&key)
                .ok()
                .flatten()
                .unwrap_or_default();

            let messages = match self.store.get_messages_since(
                jid,
                &last_agent_ts,
                &self.config.assistant_name,
            ) {
                Ok(msgs) => msgs,
                Err(e) => {
                    warn!(jid, "Failed to fetch messages: {}", e);
                    continue;
                }
            };

            if messages.is_empty() {
                continue;
            }

            if let Some(last_msg) = messages.last() {
                let _ = self.store.set_router_state(&key, &last_msg.timestamp);
            }

            let mut prompt = String::from("<messages>\n");
            for msg in &messages {
                prompt.push_str(&format!(
                    "<message sender=\"{}\" channel=\"{}\" time=\"{}\">{}</message>\n",
                    xml_escape(&msg.sender_name),
                    xml_escape(jid),
                    msg.timestamp,
                    xml_escape(&msg.content),
                ));
            }
            prompt.push_str("</messages>");

            let group_folder = group.folder.clone();
            let workspace_dir = self.config.groups_dir.join(&group_folder);
            let is_main = self.config.is_main_group(&group_folder);

            work_items.push(ModuleWorkItem {
                prompt,
                group_folder,
                addr: ChannelAddr::new(MODULE_ID, jid),
                is_main,
                is_scheduled_task: false,
                workspace_dir,
                global_claude_md: None,
            });
        }

        // 2. Process private chats (without registration)
        let private_chats = self
            .store
            .get_active_private_chats(MODULE_ID)
            .unwrap_or_default();
        for (jid, last_agent_ts) in private_chats {
            let messages = match self.store.get_messages_since(
                &jid,
                &last_agent_ts,
                &self.config.assistant_name,
            ) {
                Ok(msgs) => msgs,
                Err(e) => {
                    warn!(jid, "Failed to fetch private chat messages: {}", e);
                    continue;
                }
            };

            if messages.is_empty() {
                continue;
            }

            let key = format!("last_agent_ts:{}:{}", MODULE_ID, jid);
            if let Some(last_msg) = messages.last() {
                let _ = self.store.set_router_state(&key, &last_msg.timestamp);
            }

            let mut prompt = String::from("<messages>\n");
            for msg in &messages {
                prompt.push_str(&format!(
                    "<message sender=\"{}\" channel=\"{}\" time=\"{}\">{}</message>\n",
                    xml_escape(&msg.sender_name),
                    xml_escape(&jid),
                    msg.timestamp,
                    xml_escape(&msg.content),
                ));
            }
            prompt.push_str("</messages>");

            let group_folder = "dm".to_string();
            let workspace_dir = self.config.groups_dir.join(&group_folder);

            work_items.push(ModuleWorkItem {
                prompt,
                group_folder,
                addr: ChannelAddr::new(MODULE_ID, &jid),
                is_main: false,
                is_scheduled_task: false,
                workspace_dir,
                global_claude_md: None,
            });
        }

        Ok(work_items)
    }

    async fn shutdown(&mut self) {
        if let Some(sender) = &self.bridge_cmd_sender {
            let _ = sender.send(BridgeCommand::Shutdown).await;
        }
        self.bridge_cmd_sender = None;
    }

    async fn get_session_id(&self, group_folder: &str) -> Result<Option<String>> {
        self.store.get_session(group_folder)
    }

    async fn set_session_id(&self, group_folder: &str, session_id: &str) -> Result<()> {
        self.store.set_session(group_folder, session_id)
    }
}
