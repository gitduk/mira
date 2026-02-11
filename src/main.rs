#![allow(dead_code)]

mod agent;
mod comm;
mod config;
mod dashboard;
mod db;
mod dispatch;
mod error;
mod module;
mod modules;
mod prompt;
mod queue;
mod registry;
mod scheduler;
mod types;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tokio::sync::{mpsc, watch};
use tracing::{error, info, warn};

use comm::{
    ChannelAddr, CommunicationModule, MiraMessage, ModuleCommand, ModuleEvent, ModuleToolDefinition,
};
use config::Config;
use dashboard::{Dashboard, DashboardCommand};
use db::Database;
use module::{Module, ModuleStatus};
use prompt::xml_escape;
use queue::ModuleQueue;
use registry::{ModuleInit, ModuleRegistration};
use scheduler::Scheduler;
use types::IpcCommand;

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
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
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
    info!("State loaded");

    // Shutdown coordination
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let shutdown_tx_signal = shutdown_tx.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = shutdown_tx_signal.send(true);
        }
    });

    // IPC channel: agent tools -> main loop
    let (ipc_tx, mut ipc_rx) = mpsc::channel::<IpcCommand>(100);

    // Dashboard channel
    let (dash_tx, dash_rx) = mpsc::channel::<DashboardCommand>(200);
    let (input_tx, mut input_rx) = mpsc::channel::<String>(10);

    // Unified module event channel
    let (module_event_tx, mut module_event_rx) = mpsc::channel::<ModuleEvent>(200);

    // ModuleQueue
    let module_queue = Arc::new(ModuleQueue::new(config.max_concurrent_agents));

    // Start dashboard (if enabled)
    let _dashboard_handle = if config.dashboard_enabled {
        let dashboard = Dashboard::new(
            config.assistant_name.clone(),
            config.max_concurrent_agents,
            config.api_base_url.clone(),
            config.claude_model.clone(),
            config.dashboard_max_lines,
            dash_rx,
            shutdown_rx.clone(),
            Some(input_tx.clone()),
        );
        Some(std::thread::spawn(move || {
            if let Err(e) = dashboard.run() {
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

    // Enable input by default
    let _ = dash_tx.send(DashboardCommand::EnableInputMode).await;

    // --- Module registry ---
    let modules: Arc<tokio::sync::Mutex<HashMap<String, Box<dyn CommunicationModule>>>> =
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    // Start modules from registry
    let base_dir = std::env::current_dir().unwrap_or_default();
    let mut init = ModuleInit {
        config: config.clone(),
        db: db.clone(),
        dash_tx: dash_tx.clone(),
        base_dir,
    };

    let mut module_displays = Vec::new();
    for reg in inventory::iter::<ModuleRegistration> {
        let mut display = Module::new(reg.id, reg.display_name);
        display.mounted = true;

        let readiness = (reg.ready)(&init);
        if !readiness.ready {
            display.status = ModuleStatus::Disconnected;
            module_displays.push(display);
            warn!(
                module_id = reg.id,
                reason = readiness.reason.as_deref().unwrap_or("not ready"),
                "Module skipped"
            );
            continue;
        }

        display.status = reg.initial_status.clone();
        module_displays.push(display);

        let mut module = (reg.build)(&mut init);
        match module.start().await {
            Ok(event_rx) => {
                let tx = module_event_tx.clone();
                tokio::spawn(async move {
                    let mut rx = event_rx;
                    while let Some(event) = rx.recv().await {
                        if tx.send(event).await.is_err() {
                            break;
                        }
                    }
                });
                info!(module_id = reg.id, "Module started");
            }
            Err(e) => {
                error!(module_id = reg.id, "Failed to start module: {}", e);
            }
        }

        let mut modules_lock = modules.lock().await;
        modules_lock.insert(reg.id.into(), module);
    }

    let _ = dash_tx
        .send(DashboardCommand::SetModules(module_displays))
        .await;

    // Core terminal input queue (not a module)
    let core_pending: Arc<tokio::sync::Mutex<VecDeque<MiraMessage>>> =
        Arc::new(tokio::sync::Mutex::new(VecDeque::new()));

    {
        let core_pending = core_pending.clone();
        let dash_tx = dash_tx.clone();
        let module_queue = module_queue.clone();
        let config = config.clone();
        tokio::spawn(async move {
            while let Some(input) = input_rx.recv().await {
                let _ = dash_tx
                    .send(DashboardCommand::PushThinkingLine(format!(
                        "[TERM] {}",
                        input
                    )))
                    .await;
                let _ = dash_tx.send(DashboardCommand::IncrementMessages(1)).await;
                let timestamp = chrono::Utc::now().to_rfc3339();
                let msg = MiraMessage {
                    addr: ChannelAddr::new("terminal", "terminal"),
                    msg_id: format!("term-{}", chrono::Utc::now().timestamp_millis()),
                    sender_id: "owner".into(),
                    sender_name: config.owner_name.clone(),
                    content: input,
                    timestamp,
                    is_from_self: false,
                    channel_name: Some("Terminal".into()),
                };
                {
                    let mut pending = core_pending.lock().await;
                    pending.push_back(msg);
                }
                module_queue.enqueue_module("core".into()).await;
            }
        });
    }

    // Wire up ModuleQueue process_fn — modules emit work items, Mira runs agents.
    {
        let process_db = db.clone();
        let process_config = config.clone();
        let process_ipc_tx = ipc_tx.clone();
        let process_modules = modules.clone();
        let process_dash_tx = dash_tx.clone();
        let core_pending = core_pending.clone();

        module_queue
            .set_process_fn(Arc::new(move |module_id: String| {
                let db = process_db.clone();
                let config = process_config.clone();
                let ipc_tx = process_ipc_tx.clone();
                let modules = process_modules.clone();
                let dash_tx = process_dash_tx.clone();
                let core_pending = core_pending.clone();

                Box::pin(async move {
                    if module_id == "core" {
                        let mut pending = core_pending.lock().await;
                        if pending.is_empty() {
                            return true;
                        }

                        let mut prompt = String::from("<messages>\n");
                        while let Some(msg) = pending.pop_front() {
                            prompt.push_str(&format!(
                                "<message sender=\"{}\" channel=\"terminal\" time=\"{}\">{}</message>\n",
                                xml_escape(&msg.sender_name),
                                msg.timestamp,
                                xml_escape(&msg.content),
                            ));
                        }
                        prompt.push_str("</messages>");

                        let tools_info = {
                            let mut modules_lock = modules.lock().await;
                            let mut tools_by_module = Vec::new();
                            for (mid, module) in modules_lock.iter_mut() {
                                tools_by_module.push((mid.clone(), module.tools()));
                            }
                            format_all_module_tools(tools_by_module)
                        };
                        let prompt = format!("{}\n{}", tools_info, prompt);

                        let agent_input = agent::AgentInput {
                            prompt,
                            session_id: None,
                            group_folder: "terminal".to_string(),
                            addr: ChannelAddr::new("terminal", "terminal"),
                            is_main: true,
                            is_scheduled_task: false,
                            persist_session: false,
                            workspace_dir: config.store_dir.clone(),
                            global_claude_md: None,
                        };

                        let dash_tx_clone = dash_tx.clone();
                        let on_progress: Option<agent::OnProgressCallback> =
                            Some(Box::new(move |event| match event.event_type {
                                agent::ProgressEventType::TextStreaming => {
                                    let _ = dash_tx_clone
                                        .try_send(DashboardCommand::SetThinkingIndicator(None));
                                    let _ = dash_tx_clone.try_send(
                                        DashboardCommand::SetStreamingLine(Some(event.text)),
                                    );
                                }
                                agent::ProgressEventType::Text => {
                                    let _ = dash_tx_clone
                                        .try_send(DashboardCommand::SetThinkingIndicator(None));
                                    let _ = dash_tx_clone
                                        .try_send(DashboardCommand::SetStreamingLine(None));
                                    let _ = dash_tx_clone
                                        .try_send(DashboardCommand::PushThinkingLine(event.text));
                                }
                                agent::ProgressEventType::ToolUse => {
                                    let _ = dash_tx_clone
                                        .try_send(DashboardCommand::SetStreamingLine(None));
                                    let _ = dash_tx_clone.try_send(
                                        DashboardCommand::SetThinkingIndicator(Some(event.text)),
                                    );
                                }
                                agent::ProgressEventType::ToolSummary => {
                                    let _ = dash_tx_clone
                                        .try_send(DashboardCommand::PushThinkingLine(event.text));
                                }
                            }));

                        let _ = dash_tx
                            .send(DashboardCommand::SetThinkingIndicator(Some(
                                "Thinking...".into(),
                            )))
                            .await;

                        let output = agent::run_agent(
                            agent_input,
                            &config,
                            db.clone(),
                            ipc_tx.clone(),
                            on_progress,
                        )
                        .await;

                        let _ = dash_tx
                            .send(DashboardCommand::SetThinkingIndicator(None))
                            .await;

                        if output.status == agent::AgentStatus::Error {
                            error!(
                                module_id = %module_id,
                                error = ?output.error,
                                "Agent failed for core input"
                            );
                            let _ = dash_tx
                                .send(DashboardCommand::PushThinkingLine(format!(
                                    "Error: {}",
                                    output.error.unwrap_or_default()
                                )))
                                .await;
                            return false;
                        }

                        return true;
                    }

                    let work_items = {
                        let mut modules_lock = modules.lock().await;
                        let Some(module) = modules_lock.get_mut(&module_id) else {
                            warn!(module_id, "No module registered for dispatch");
                            return false;
                        };
                        match module.drain_work_items().await {
                            Ok(items) => items,
                            Err(e) => {
                                error!(module_id, "Failed to drain work items: {}", e);
                                return false;
                            }
                        }
                    };

                    if work_items.is_empty() {
                        return true;
                    }

                    for item in work_items {
                        let (session_id, persist_session) = {
                            let mut modules_lock = modules.lock().await;
                            if let Some(module) = modules_lock.get_mut(&module_id) {
                                let sid = module
                                    .get_session_id(&item.group_folder)
                                    .await
                                    .ok()
                                    .flatten();
                                (sid, module.persist_sessions())
                            } else {
                                (None, true)
                            }
                        };

                        let tools_info = {
                            let mut modules_lock = modules.lock().await;
                            let mut tools_by_module = Vec::new();
                            for (mid, module) in modules_lock.iter_mut() {
                                tools_by_module.push((mid.clone(), module.tools()));
                            }
                            format_all_module_tools(tools_by_module)
                        };
                        let prompt = format!("{}\n{}", tools_info, item.prompt);

                        let agent_input = agent::AgentInput {
                            prompt,
                            session_id,
                            group_folder: item.group_folder.clone(),
                            addr: item.addr.clone(),
                            is_main: item.is_main,
                            is_scheduled_task: item.is_scheduled_task,
                            persist_session,
                            workspace_dir: item.workspace_dir,
                            global_claude_md: item.global_claude_md,
                        };

                        let dash_tx_clone = dash_tx.clone();
                        let on_progress: Option<agent::OnProgressCallback> =
                            Some(Box::new(move |event| match event.event_type {
                                agent::ProgressEventType::TextStreaming => {
                                    let _ = dash_tx_clone
                                        .try_send(DashboardCommand::SetThinkingIndicator(None));
                                    let _ = dash_tx_clone.try_send(
                                        DashboardCommand::SetStreamingLine(Some(event.text)),
                                    );
                                }
                                agent::ProgressEventType::Text => {
                                    let _ = dash_tx_clone
                                        .try_send(DashboardCommand::SetThinkingIndicator(None));
                                    let _ = dash_tx_clone
                                        .try_send(DashboardCommand::SetStreamingLine(None));
                                    let _ = dash_tx_clone
                                        .try_send(DashboardCommand::PushThinkingLine(event.text));
                                }
                                agent::ProgressEventType::ToolUse => {
                                    let _ = dash_tx_clone
                                        .try_send(DashboardCommand::SetStreamingLine(None));
                                    let _ = dash_tx_clone.try_send(
                                        DashboardCommand::SetThinkingIndicator(Some(event.text)),
                                    );
                                }
                                agent::ProgressEventType::ToolSummary => {
                                    let _ = dash_tx_clone
                                        .try_send(DashboardCommand::PushThinkingLine(event.text));
                                }
                            }));

                        let _ = dash_tx
                            .send(DashboardCommand::SetThinkingIndicator(Some(
                                "Thinking...".into(),
                            )))
                            .await;

                        // Send "composing" presence to show typing indicator
                        {
                            let modules_lock = modules.lock().await;
                            if let Some(module) = modules_lock.get(&module_id) {
                                let _ = module
                                    .send_command(ModuleCommand::SendPresence {
                                        channel_id: item.addr.channel_id.clone(),
                                        presence: "composing".into(),
                                    })
                                    .await;
                            }
                        }

                        let output = agent::run_agent(
                            agent_input,
                            &config,
                            db.clone(),
                            ipc_tx.clone(),
                            on_progress,
                        )
                        .await;

                        // Clear typing indicator
                        {
                            let modules_lock = modules.lock().await;
                            if let Some(module) = modules_lock.get(&module_id) {
                                let _ = module
                                    .send_command(ModuleCommand::SendPresence {
                                        channel_id: item.addr.channel_id.clone(),
                                        presence: "paused".into(),
                                    })
                                    .await;
                            }
                        }

                        let _ = dash_tx
                            .send(DashboardCommand::SetThinkingIndicator(None))
                            .await;

                        if let Some(sid) = output.new_session_id {
                            let mut modules_lock = modules.lock().await;
                            if let Some(module) = modules_lock.get_mut(&module_id) {
                                let _ = module.set_session_id(&item.group_folder, &sid).await;
                            }
                        }

                        if output.status == agent::AgentStatus::Error {
                            error!(
                                module_id = %module_id,
                                error = ?output.error,
                                "Agent failed for module"
                            );
                            let _ = dash_tx
                                .send(DashboardCommand::PushThinkingLine(format!(
                                    "Error: {}",
                                    output.error.unwrap_or_default()
                                )))
                                .await;
                            return false;
                        }

                        // Send agent's text reply back to the originating channel
                        if let Some(ref resp) = output.result {
                            if let Some(ref text) = resp.user_message {
                                if !text.trim().is_empty() {
                                    let _ = ipc_tx
                                        .send(IpcCommand::SendMessage {
                                            addr: item.addr.clone(),
                                            text: text.clone(),
                                        })
                                        .await;
                                }
                            }
                        }
                    }

                    true
                })
            }))
            .await;
    }

    // Start scheduler
    let scheduler_db = db.clone();
    let scheduler_config = config.clone();
    let scheduler_shutdown_rx = shutdown_rx.clone();
    let scheduler_handle = tokio::spawn(async move {
        let mut scheduler = Scheduler::new(scheduler_db, scheduler_config);
        scheduler.run_loop(scheduler_shutdown_rx).await;
    });

    // Main event loop
    loop {
        tokio::select! {
            // Unified module events
            Some(event) = module_event_rx.recv() => {
                match event {
                    ModuleEvent::Message(msg) => {
                        if !msg.is_from_self {
                            let display = format!(
                                "[{}] {}: {}",
                                msg.addr.module_id,
                                msg.sender_name,
                                msg.content,
                            );
                            let _ = dash_tx
                                .send(DashboardCommand::PushThinkingLine(display))
                                .await;
                            let _ = dash_tx
                                .send(DashboardCommand::IncrementMessages(1))
                                .await;
                            module_queue
                                .enqueue_module(msg.addr.module_id.clone())
                                .await;
                        }
                    }
                    ModuleEvent::StatusChange { module_id, status } => {
                        let _ = dash_tx.send(DashboardCommand::SetModuleStatus(
                            module_id, status
                        )).await;
                    }
                    ModuleEvent::Log { module_id, message } => {
                        let _ = dash_tx
                            .send(DashboardCommand::PushThinkingLine(format!(
                                "[{}] {}",
                                module_id, message
                            )))
                            .await;
                    }
                    ModuleEvent::Error { module_id, message } => {
                        error!(module_id, message, "Module error");
                    }
                }
            }

            // IPC commands from agent tools
            Some(cmd) = ipc_rx.recv() => {
                handle_ipc_command(
                    cmd, &config, &db,
                    &dash_tx, &modules,
                ).await;
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

    // Graceful shutdown
    info!("Shutting down...");
    module_queue.shutdown(10_000).await;
    {
        let mut modules_lock = modules.lock().await;
        for (id, module) in modules_lock.iter_mut() {
            info!(module_id = %id, "Shutting down module");
            module.shutdown().await;
        }
    }
    scheduler_handle.abort();
    info!("Mira shutdown complete");
}

async fn handle_ipc_command(
    cmd: IpcCommand,
    config: &Config,
    db: &Arc<Database>,
    dash_tx: &mpsc::Sender<DashboardCommand>,
    modules: &Arc<tokio::sync::Mutex<HashMap<String, Box<dyn CommunicationModule>>>>,
) {
    match cmd {
        IpcCommand::CallModuleTool {
            module_id,
            tool_name,
            input,
        } => {
            let result = {
                let mut modules_lock = modules.lock().await;
                if let Some(module) = modules_lock.get_mut(&module_id) {
                    module
                        .call_tool(crate::comm::ModuleToolCall {
                            name: tool_name.clone(),
                            input: input.clone(),
                        })
                        .await
                        .map_err(|e| e.to_string())
                } else {
                    Err("Module not found".to_string())
                }
            };
            match result {
                Ok(res) => {
                    let _ = dash_tx
                        .send(DashboardCommand::PushThinkingLine(res.content))
                        .await;
                }
                Err(e) => {
                    let _ = dash_tx
                        .send(DashboardCommand::PushThinkingLine(format!(
                            "Tool error: {}",
                            e
                        )))
                        .await;
                }
            }
        }
        IpcCommand::SendMessage { addr, text } => {
            info!(%addr, "IPC: send_message");
            if addr.module_id == "terminal" {
                let _ = dash_tx.send(DashboardCommand::PushThinkingLine(text)).await;
                return;
            }
            let mut modules_lock = modules.lock().await;
            if let Some(module) = modules_lock.get_mut(&addr.module_id) {
                if let Err(e) = module
                    .send_command(ModuleCommand::SendMessage {
                        channel_id: addr.channel_id,
                        text,
                    })
                    .await
                {
                    error!(%addr.module_id, "Failed to send message via module: {}", e);
                }
            } else {
                warn!(%addr, "No module found for send_message");
            }
        }
        IpcCommand::ScheduleTask {
            prompt,
            schedule_type,
            schedule_value,
            context_mode,
            target_addr,
            source_group,
        } => {
            if let Some(module) = modules.lock().await.get_mut(&target_addr.module_id) {
                let has_auth_tool = module
                    .tools()
                    .iter()
                    .any(|t| t.name == "authorize_schedule_task");
                if has_auth_tool {
                    let result = module
                        .call_tool(crate::comm::ModuleToolCall {
                            name: "authorize_schedule_task".into(),
                            input: serde_json::json!({
                                "source_group": source_group,
                                "target_jid": target_addr.channel_id
                            }),
                        })
                        .await
                        .map_err(|e| e.to_string());
                    if let Ok(res) = result {
                        if res.is_error {
                            warn!(%target_addr, "Unauthorized schedule_task blocked");
                            return;
                        }
                    }
                }
            }

            let is_main = config.is_main_group(&source_group);
            let task_id = format!(
                "task-{}-{}",
                chrono::Utc::now().timestamp_millis(),
                &uuid::Uuid::new_v4().to_string()[..6]
            );
            let now = chrono::Utc::now().to_rfc3339();
            let target_folder = if is_main {
                source_group.clone()
            } else {
                source_group.clone()
            };

            let stype =
                types::ScheduleType::from_str(&schedule_type).unwrap_or(types::ScheduleType::Once);
            let cmode = types::ContextMode::from_str(&context_mode);

            let task = types::ScheduledTask {
                id: task_id.clone(),
                group_folder: target_folder,
                chat_jid: target_addr.channel_id.clone(),
                prompt,
                schedule_type: stype,
                schedule_value,
                context_mode: cmode,
                next_run: None,
                last_run: None,
                last_result: None,
                status: types::TaskStatus::Active,
                created_at: now,
                module_id: target_addr.module_id,
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
        IpcCommand::PauseTask {
            task_id,
            source_group,
        } => {
            let is_main = config.is_main_group(&source_group);
            if let Ok(Some(task)) = db.get_task_by_id(&task_id) {
                if is_main || task.group_folder == source_group {
                    let _ = db.update_task_status(&task_id, &types::TaskStatus::Paused);
                    info!(%task_id, "Task paused via IPC");
                }
            }
        }
        IpcCommand::ResumeTask {
            task_id,
            source_group,
        } => {
            let is_main = config.is_main_group(&source_group);
            if let Ok(Some(task)) = db.get_task_by_id(&task_id) {
                if is_main || task.group_folder == source_group {
                    let _ = db.update_task_status(&task_id, &types::TaskStatus::Active);
                    info!(%task_id, "Task resumed via IPC");
                }
            }
        }
        IpcCommand::CancelTask {
            task_id,
            source_group,
        } => {
            let is_main = config.is_main_group(&source_group);
            if let Ok(Some(task)) = db.get_task_by_id(&task_id) {
                if is_main || task.group_folder == source_group {
                    let _ = db.delete_task(&task_id);
                    info!(%task_id, "Task cancelled via IPC");
                }
            }
        }
    }
}

fn format_all_module_tools(tools_by_module: Vec<(String, Vec<ModuleToolDefinition>)>) -> String {
    let mut out = String::new();
    out.push_str("[module_tools]\n");
    out.push_str("Tools (call via module_tool with module_id/tool_name):\n");

    let mut any = false;
    for (module_id, tools) in tools_by_module {
        for t in tools {
            any = true;
            let schema = serde_json::to_string(&t.input_schema).unwrap_or_else(|_| "{}".into());
            out.push_str(&format!("- name: {}.{}\n", module_id, t.name));
            out.push_str(&format!("  desc: {}\n", t.description));
            out.push_str(&format!("  schema: {}\n", schema));
        }
    }

    if !any {
        out.push_str("none\n");
    }

    out.push_str("[/module_tools]\n");
    out
}
