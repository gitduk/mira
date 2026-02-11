pub mod bash;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod ipc;
pub mod read;
pub mod web_fetch;
pub mod web_search;
pub mod write;

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::agent::api::ToolDefinition;
use crate::comm::ChannelAddr;
use crate::db::Database;
use crate::types::IpcCommand;

pub struct ToolContext {
    pub workspace_dir: PathBuf,
    pub group_folder: String,
    pub addr: ChannelAddr,
    pub is_main: bool,
    pub ipc_sender: mpsc::Sender<IpcCommand>,
    pub db: Arc<Database>,
}

pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn ok(content: String) -> Self {
        ToolResult {
            content,
            is_error: false,
        }
    }

    pub fn error(content: String) -> Self {
        ToolResult {
            content,
            is_error: true,
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::error::Result<ToolResult>;
}

pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
    ctx: ToolContext,
}

impl ToolRegistry {
    pub fn new(ctx: ToolContext) -> Self {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(bash::BashTool),
            Box::new(read::ReadTool),
            Box::new(write::WriteTool),
            Box::new(edit::EditTool),
            Box::new(glob::GlobTool),
            Box::new(grep::GrepTool),
            Box::new(web_search::WebSearchTool),
            Box::new(web_fetch::WebFetchTool),
            Box::new(ipc::SendMessageTool),
            Box::new(ipc::ScheduleTaskTool),
            Box::new(ipc::ListTasksTool),
            Box::new(ipc::PauseTaskTool),
            Box::new(ipc::ResumeTaskTool),
            Box::new(ipc::CancelTaskTool),
            Box::new(ipc::ModuleToolCallTool),
        ];

        ToolRegistry { tools, ctx }
    }

    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .iter()
            .map(|t| ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect()
    }

    pub async fn execute(&self, name: &str, input: Value) -> crate::error::Result<ToolResult> {
        for tool in &self.tools {
            if tool.name() == name {
                return tool.execute(input, &self.ctx).await;
            }
        }
        Ok(ToolResult::error(format!("Unknown tool: {}", name)))
    }
}
