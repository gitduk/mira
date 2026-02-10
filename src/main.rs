#![allow(dead_code)]

mod agent;
mod bridge;
mod config;
mod dashboard;
mod db;
mod error;
mod module;
mod queue;
mod scheduler;
mod types;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::{mpsc, watch};
use tracing::{error, info, warn};

use bridge::protocol::BridgeEvent;
use config::Config;
use dashboard::state::{GroupDisplay, GroupDisplayStatus};
use dashboard::{Dashboard, DashboardCommand};
use db::Database;
use module::{Module, ModuleStatus};
use queue::GroupQueue;
use scheduler::Scheduler;
use types::{IpcCommand, RegisteredGroup};

#[tokio::main]
async fn main() {
    // Initialize config
    let config = Arc::new(Config::from_env());

    // Initialize tracing — write to file when dashboard is active to avoid corrupting the TUI
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.log_level));
    if config.dashboard_enabled {
        let _ = std::fs::create_dir_all(&config.store_dir);
        let log_file = std::fs::File::create(config.store_dir.join("mira.log"))
            .expect("Failed to create log file");
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_writer(std::sync::Mutex::new(log_file))
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .init();
    }

    info!("Mira starting...");

    // Initialize database
    let db = match Database::open(&config.store_dir) {
        Ok(db) => Arc::new(db),
        Err(e) => {
            error!("Failed to open database: {}", e);
            std::process::exit(1);
        }
    };
    info!("Database initialized");

    // Load state
    let mut registered_groups = db.get_all_registered_groups().unwrap_or_default();
    let mut sessions = db.get_all_sessions().unwrap_or_default();
    let mut last_timestamp = db
        .get_router_state("last_timestamp")
        .ok()
        .flatten()
        .unwrap_or_default();
    let _last_agent_timestamp: HashMap<String, String> = db
        .get_router_state("last_agent_timestamp")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    info!(
        group_count = registered_groups.len(),
        "State loaded"
    );

    // Shutdown coordination
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // IPC channel: agent tools -> main loop
    let (ipc_tx, mut ipc_rx) = mpsc::channel::<IpcCommand>(100);

    // Dashboard channel
    let (dash_tx, dash_rx) = mpsc::channel::<DashboardCommand>(200);
    let (input_tx, mut input_rx) = mpsc::channel::<String>(10);

    // Bridge events channel
    let (bridge_event_tx, mut bridge_event_rx) = mpsc::channel::<BridgeEvent>(100);

    // GroupQueue
    let group_queue = Arc::new(GroupQueue::new(config.max_concurrent_agents));

    // Start dashboard (if enabled)
    let _dashboard_handle = if config.dashboard_enabled {
        let dashboard = Dashboard::new(
            config.assistant_name.clone(),
            config.max_concurrent_agents,
            config.api_base_url.clone(),
            config.claude_model.clone(),
            dash_rx,
            shutdown_rx.clone(),
            Some(input_tx.clone()),
        );
        Some(tokio::spawn(async move {
            if let Err(e) = dashboard.run().await {
                error!("Dashboard error: {}", e);
            }
        }))
    } else {
        // Still consume the receiver
        tokio::spawn(async move {
            let mut rx = dash_rx;
            while rx.recv().await.is_some() {}
        });
        None
    };

    // Send initial groups to dashboard
    {
        let groups: HashMap<String, GroupDisplay> = registered_groups
            .iter()
            .map(|(jid, g)| {
                (
                    jid.clone(),
                    GroupDisplay {
                        name: g.name.clone(),
                        folder: g.folder.clone(),
                        status: GroupDisplayStatus::Idle,
                    },
                )
            })
            .collect();
        let _ = dash_tx.send(DashboardCommand::SetGroups(groups)).await;
        let _ = dash_tx.send(DashboardCommand::EnableInputMode).await;

        // Initialize modules
        let mut whatsapp = Module::new("whatsapp", "WhatsApp");
        whatsapp.mounted = true;
        whatsapp.status = ModuleStatus::Connecting;
        let _ = dash_tx.send(DashboardCommand::SetModules(vec![whatsapp])).await;
    }

    // Start bridge manager
    let bridge_dir = std::env::current_dir()
        .unwrap_or_default()
        .join("bridge");
    let store_dir = config.store_dir.clone();
    let bridge_event_tx_clone = bridge_event_tx.clone();
    let bridge_shutdown_rx = shutdown_rx.clone();

    let bridge_handle = tokio::spawn(async move {
        let mut manager =
            bridge::BridgeManager::new(bridge_dir, store_dir, bridge_event_tx_clone);
        tokio::select! {
            result = manager.run() => {
                if let Err(e) = result {
                    error!("Bridge manager error: {}", e);
                }
            }
            _ = async {
                let mut rx = bridge_shutdown_rx;
                loop {
                    if rx.changed().await.is_err() { break; }
                    if *rx.borrow() { break; }
                }
            } => {
                manager.request_shutdown();
            }
        }
    });

    // Start scheduler
    let scheduler_db = db.clone();
    let scheduler_config = config.clone();
    let scheduler_shutdown_rx = shutdown_rx.clone();
    let scheduler_handle = tokio::spawn(async move {
        let mut scheduler = Scheduler::new(scheduler_db, scheduler_config);
        scheduler.run_loop(scheduler_shutdown_rx).await;
    });

    // Graceful shutdown handler
    let shutdown_tx_clone = shutdown_tx.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Shutdown signal received");
        let _ = shutdown_tx_clone.send(true);
    });

    // Main event loop
    let mut message_poll_interval =
        tokio::time::interval(tokio::time::Duration::from_millis(config.poll_interval_ms));

    loop {
        tokio::select! {
            // Message polling
            _ = message_poll_interval.tick() => {
                if *shutdown_rx.borrow() {
                    break;
                }

                let jids: Vec<String> = registered_groups.keys().cloned().collect();
                match db.get_new_messages(&jids, &last_timestamp, &config.assistant_name) {
                    Ok((messages, new_ts)) => {
                        if !messages.is_empty() {
                            let _ = dash_tx.send(DashboardCommand::IncrementMessages(messages.len())).await;
                            info!(count = messages.len(), "New messages");

                            last_timestamp = new_ts;
                            let _ = db.set_router_state("last_timestamp", &last_timestamp);

                            let mut groups_with_messages = HashSet::new();
                            for msg in &messages {
                                groups_with_messages.insert(msg.chat_jid.clone());
                            }

                            for chat_jid in groups_with_messages {
                                group_queue.enqueue_message_check(chat_jid).await;
                            }
                        }
                    }
                    Err(e) => {
                        error!("Error polling messages: {}", e);
                    }
                }
            }

            // Bridge events
            Some(event) = bridge_event_rx.recv() => {
                match event {
                    BridgeEvent::Ready { user_jid, lid_jid } => {
                        info!(?user_jid, ?lid_jid, "Bridge ready");
                        let _ = dash_tx.send(DashboardCommand::SetModuleStatus("whatsapp".into(), ModuleStatus::Connected)).await;
                    }
                    BridgeEvent::Message { msg_id, chat_jid, sender, sender_name, content, timestamp, is_from_me, chat_name } => {
                        let _ = db.store_chat_metadata(&chat_jid, &timestamp, chat_name.as_deref());
                        if registered_groups.contains_key(&chat_jid) {
                            let _ = db.store_message(&msg_id, &chat_jid, &sender, &sender_name, &content, &timestamp, is_from_me);
                        }
                    }
                    BridgeEvent::ChatMetadata { chat_jid, timestamp, name } => {
                        let _ = db.store_chat_metadata(&chat_jid, &timestamp, name.as_deref());
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
                        let _ = dash_tx.send(DashboardCommand::SetModuleStatus("whatsapp".into(), module_status)).await;
                    }
                    BridgeEvent::GroupsResult { groups } => {
                        for g in groups {
                            let _ = db.update_chat_name(&g.jid, &g.subject);
                        }
                        let _ = db.set_last_group_sync();
                    }
                    BridgeEvent::Error { message } => {
                        error!("Bridge error: {}", message);
                    }
                    BridgeEvent::Qr { .. } => {
                        error!("WhatsApp QR required - run setup first");
                    }
                    BridgeEvent::MessageSent { .. } => {}
                }
            }

            // IPC commands from agent tools
            Some(cmd) = ipc_rx.recv() => {
                handle_ipc_command(cmd, &config, &db, &mut registered_groups, &mut sessions, &dash_tx).await;
            }

            // Terminal input
            Some(input) = input_rx.recv() => {
                let _ = dash_tx.send(DashboardCommand::PushThinkingLine(
                    format!("[TERM] {}", input)
                )).await;

                let (workspace_dir, group_folder, chat_jid, is_main) =
                    if let Some((jid, group)) = registered_groups.iter().next() {
                        (config.groups_dir.join(&group.folder), group.folder.clone(),
                         jid.clone(), config.is_main_group(&group.folder))
                    } else {
                        let folder = "terminal".to_string();
                        (config.groups_dir.join(&folder), folder, "terminal".to_string(), true)
                    };

                let timestamp = chrono::Utc::now().to_rfc3339();
                let prompt = format!(
                    "<messages>\n<message sender=\"{}\" channel=\"terminal\" time=\"{}\">{}</message>\n</messages>",
                    xml_escape(&config.owner_name),
                    timestamp,
                    xml_escape(&input)
                );

                let agent_input = agent::AgentInput {
                    prompt,
                    session_id: sessions.get(&group_folder).cloned(),
                    group_folder: group_folder.clone(),
                    chat_jid,
                    is_main,
                    is_scheduled_task: false,
                    workspace_dir,
                    global_claude_md: None,
                };

                let dash_tx_clone = dash_tx.clone();
                let on_progress: Option<agent::OnProgressCallback> = Some(Box::new(move |event| {
                    match event.event_type {
                        agent::ProgressEventType::TextStreaming => {
                            let _ = dash_tx_clone.try_send(DashboardCommand::SetThinkingIndicator(None));
                            let _ = dash_tx_clone.try_send(DashboardCommand::SetStreamingLine(Some(event.text)));
                        }
                        agent::ProgressEventType::Text => {
                            let _ = dash_tx_clone.try_send(DashboardCommand::SetThinkingIndicator(None));
                            let _ = dash_tx_clone.try_send(DashboardCommand::SetStreamingLine(None));
                            let _ = dash_tx_clone.try_send(DashboardCommand::PushThinkingLine(event.text));
                        }
                        agent::ProgressEventType::ToolUse => {
                            let _ = dash_tx_clone.try_send(DashboardCommand::SetStreamingLine(None));
                            let _ = dash_tx_clone.try_send(DashboardCommand::SetThinkingIndicator(Some(event.text)));
                        }
                        agent::ProgressEventType::ToolSummary => {
                            let _ = dash_tx_clone.try_send(DashboardCommand::PushThinkingLine(event.text));
                        }
                    }
                }));

                let _ = dash_tx.send(DashboardCommand::SetThinkingIndicator(
                    Some("Thinking...".into())
                )).await;

                let output = agent::run_agent(
                    agent_input,
                    &config,
                    db.clone(),
                    ipc_tx.clone(),
                    on_progress,
                ).await;

                let _ = dash_tx.send(DashboardCommand::SetThinkingIndicator(None)).await;

                if let Some(sid) = output.new_session_id {
                    sessions.insert(group_folder.clone(), sid.clone());
                    let _ = db.set_session(&group_folder, &sid);
                }

                if output.status == agent::AgentStatus::Error {
                    let _ = dash_tx.send(DashboardCommand::PushThinkingLine(
                        format!("Error: {}", output.error.unwrap_or_default())
                    )).await;
                }
            }

            // Shutdown
            _ = async {
                let mut rx = shutdown_rx.clone();
                loop {
                    if rx.changed().await.is_err() { break; }
                    if *rx.borrow() { break; }
                }
            } => {
                break;
            }
        }
    }

    info!("Shutting down...");
    group_queue.shutdown(10_000).await;
    bridge_handle.abort();
    scheduler_handle.abort();
    info!("Mira shutdown complete");
}

async fn handle_ipc_command(
    cmd: IpcCommand,
    config: &Config,
    db: &Arc<Database>,
    registered_groups: &mut HashMap<String, RegisteredGroup>,
    _sessions: &mut HashMap<String, String>,
    dash_tx: &mpsc::Sender<DashboardCommand>,
) {
    match cmd {
        IpcCommand::SendMessage { chat_jid, text: _ } => {
            info!(chat_jid, "IPC: send_message");
            // Bridge send will be implemented when bridge command forwarding is connected
        }
        IpcCommand::ScheduleTask { prompt, schedule_type, schedule_value, context_mode, target_jid, source_group } => {
            let is_main = config.is_main_group(&source_group);
            if !is_main {
                if let Some(target) = registered_groups.get(&target_jid) {
                    if target.folder != source_group {
                        warn!(source_group, target_jid, "Unauthorized schedule_task blocked");
                        return;
                    }
                }
            }

            let task_id = format!("task-{}-{}", chrono::Utc::now().timestamp_millis(), &uuid::Uuid::new_v4().to_string()[..6]);
            let now = chrono::Utc::now().to_rfc3339();
            let target_folder = registered_groups.get(&target_jid)
                .map(|g| g.folder.clone())
                .unwrap_or(source_group);

            let stype = types::ScheduleType::from_str(&schedule_type).unwrap_or(types::ScheduleType::Once);
            let cmode = types::ContextMode::from_str(&context_mode);

            let task = types::ScheduledTask {
                id: task_id.clone(),
                group_folder: target_folder,
                chat_jid: target_jid,
                prompt,
                schedule_type: stype,
                schedule_value,
                context_mode: cmode,
                next_run: None,
                last_run: None,
                last_result: None,
                status: types::TaskStatus::Active,
                created_at: now,
            };

            let next_run = scheduler::calculate_next_run(&task);
            let mut task = task;
            task.next_run = next_run;

            if let Err(e) = db.create_task(&task) {
                error!(%task_id, "Failed to create task: {}", e);
            } else {
                info!(%task_id, "Task created via IPC");
            }
        }
        IpcCommand::PauseTask { task_id, source_group } => {
            let is_main = config.is_main_group(&source_group);
            if let Ok(Some(task)) = db.get_task_by_id(&task_id) {
                if is_main || task.group_folder == source_group {
                    let _ = db.update_task_status(&task_id, &types::TaskStatus::Paused);
                    info!(%task_id, "Task paused via IPC");
                }
            }
        }
        IpcCommand::ResumeTask { task_id, source_group } => {
            let is_main = config.is_main_group(&source_group);
            if let Ok(Some(task)) = db.get_task_by_id(&task_id) {
                if is_main || task.group_folder == source_group {
                    let _ = db.update_task_status(&task_id, &types::TaskStatus::Active);
                    info!(%task_id, "Task resumed via IPC");
                }
            }
        }
        IpcCommand::CancelTask { task_id, source_group } => {
            let is_main = config.is_main_group(&source_group);
            if let Ok(Some(task)) = db.get_task_by_id(&task_id) {
                if is_main || task.group_folder == source_group {
                    let _ = db.delete_task(&task_id);
                    info!(%task_id, "Task cancelled via IPC");
                }
            }
        }
        IpcCommand::RegisterGroup { jid, name, folder, trigger } => {
            let group = RegisteredGroup {
                name: name.clone(),
                folder: folder.clone(),
                trigger,
                added_at: chrono::Utc::now().to_rfc3339(),
                requires_trigger: None,
            };
            let _ = db.set_registered_group(&jid, &group);
            registered_groups.insert(jid.clone(), group);

            let group_dir = config.groups_dir.join(&folder);
            let _ = std::fs::create_dir_all(group_dir.join("logs"));

            let groups: HashMap<String, GroupDisplay> = registered_groups
                .iter()
                .map(|(jid, g)| (jid.clone(), GroupDisplay {
                    name: g.name.clone(),
                    folder: g.folder.clone(),
                    status: GroupDisplayStatus::Idle,
                }))
                .collect();
            let _ = dash_tx.send(DashboardCommand::SetGroups(groups)).await;
            info!(jid, name, folder, "Group registered via IPC");
        }
        IpcCommand::RefreshGroups => {
            info!("Group refresh requested via IPC");
        }
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
