pub mod api;
pub mod r#loop;
pub mod session;
pub mod tools;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::error;

use crate::comm::ChannelAddr;
use crate::config::Config;
use crate::db::Database;
use crate::types::IpcCommand;

pub use self::r#loop::{AgentProgressEvent, ProgressEventType};

#[derive(Debug, Clone)]
pub struct AgentResponse {
    pub output_type: OutputType,
    pub user_message: Option<String>,
    pub internal_log: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OutputType {
    Message,
    Log,
}

pub struct AgentOutput {
    pub status: AgentStatus,
    pub result: Option<AgentResponse>,
    pub new_session_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum AgentStatus {
    Success,
    Error,
}

pub struct AgentInput {
    pub prompt: String,
    pub session_id: Option<String>,
    pub group_folder: String,
    pub addr: ChannelAddr,
    pub is_main: bool,
    pub is_scheduled_task: bool,
    pub persist_session: bool,
    pub workspace_dir: PathBuf,
    pub global_claude_md: Option<String>,
}

pub type OnProgressCallback = Box<dyn Fn(AgentProgressEvent) + Send + Sync>;

/// Run agent for a group
pub async fn run_agent(
    input: AgentInput,
    config: &Config,
    db: Arc<Database>,
    ipc_tx: mpsc::Sender<IpcCommand>,
    on_progress: Option<OnProgressCallback>,
) -> AgentOutput {
    // Prepare workspace
    if let Err(e) = std::fs::create_dir_all(&input.workspace_dir) {
        return AgentOutput {
            status: AgentStatus::Error,
            result: None,
            new_session_id: None,
            error: Some(format!("Failed to create workspace: {}", e)),
        };
    }

    // Build tool context
    let tool_ctx = tools::ToolContext {
        workspace_dir: input.workspace_dir.clone(),
        group_folder: input.group_folder.clone(),
        addr: input.addr.clone(),
        is_main: input.is_main,
        ipc_sender: ipc_tx,
        db: db.clone(),
    };

    // Build system prompt
    let system_prompt = session::build_system_prompt(
        config,
        &input.workspace_dir,
        input.global_claude_md.as_deref(),
    );

    // Add scheduled task context
    let prompt = if input.is_scheduled_task {
        format!("[SCHEDULED TASK - The following message was sent automatically and is not coming directly from the user or group.]\n\n{}", input.prompt)
    } else {
        input.prompt.clone()
    };

    // Load or create session
    let session = session::Session::load_or_create(
        &input.workspace_dir,
        input.session_id.as_deref(),
        input.persist_session,
    );

    // Run the agentic loop
    let timeout = tokio::time::Duration::from_secs(300); // 5 minutes
    let result = tokio::time::timeout(
        timeout,
        r#loop::agentic_loop(
            &config.api_base_url,
            &config.api_key,
            &config.claude_model,
            &system_prompt,
            &prompt,
            &session,
            tool_ctx,
            on_progress,
        ),
    )
    .await;

    match result {
        Ok(Ok(loop_result)) => {
            // Save session
            if input.persist_session {
                if let Err(e) = session.save(&input.workspace_dir) {
                    error!("Failed to save session: {}", e);
                }
            }

            AgentOutput {
                status: AgentStatus::Success,
                result: Some(loop_result.response),
                new_session_id: Some(session.id.clone()),
                error: None,
            }
        }
        Ok(Err(e)) => AgentOutput {
            status: AgentStatus::Error,
            result: None,
            new_session_id: Some(session.id.clone()),
            error: Some(format!("{}", e)),
        },
        Err(_) => AgentOutput {
            status: AgentStatus::Error,
            result: None,
            new_session_id: Some(session.id.clone()),
            error: Some("Agent execution timeout (5 minutes)".into()),
        },
    }
}
