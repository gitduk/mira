use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolContext, ToolResult};
use crate::comm::ChannelAddr;
use crate::types::IpcCommand;

// --- send_message ---
pub struct SendMessageTool;

#[async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Send a message to the user or group. Delivered immediately while still running."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The message text to send"
                },
                "module_id": {
                    "type": "string",
                    "description": "Target module (e.g. 'whatsapp', 'telegram'). Defaults to current module."
                },
                "channel_id": {
                    "type": "string",
                    "description": "Target channel within the module. Defaults to current channel."
                }
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::error::Result<ToolResult> {
        let text = input["text"].as_str().unwrap_or_default();
        if text.is_empty() {
            return Ok(ToolResult::error("text is required".into()));
        }

        let target_addr = ChannelAddr::new(
            input["module_id"].as_str().unwrap_or(&ctx.addr.module_id),
            input["channel_id"].as_str().unwrap_or(&ctx.addr.channel_id),
        );

        ctx.ipc_sender
            .send(IpcCommand::SendMessage {
                addr: target_addr,
                text: text.to_string(),
            })
            .await
            .map_err(|e| crate::error::MiraError::Tool(format!("Failed to send IPC: {}", e)))?;

        Ok(ToolResult::ok("Message sent.".into()))
    }
}

// --- schedule_task ---
pub struct ScheduleTaskTool;

#[async_trait]
impl Tool for ScheduleTaskTool {
    fn name(&self) -> &str {
        "schedule_task"
    }

    fn description(&self) -> &str {
        "Schedule a recurring or one-time task. The task runs as a full agent with all tools."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "What the agent should do when the task runs"
                },
                "schedule_type": {
                    "type": "string",
                    "enum": ["cron", "interval", "once"],
                    "description": "cron=recurring at specific times, interval=every N ms, once=run once"
                },
                "schedule_value": {
                    "type": "string",
                    "description": "For cron: a 7-field cron expression (sec min hour day_of_month month day_of_week year), e.g. '0 30 9 * * * *' for daily at 09:30. For interval: milliseconds. For once: ISO 8601 timestamp."
                },
                "context_mode": {
                    "type": "string",
                    "enum": ["group", "isolated"],
                    "description": "group=with chat history, isolated=fresh session"
                },
                "target_module_id": {
                    "type": "string",
                    "description": "Target module (main only, defaults to current)"
                },
                "target_channel_id": {
                    "type": "string",
                    "description": "Target channel (main only, defaults to current)"
                }
            },
            "required": ["prompt", "schedule_type", "schedule_value"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::error::Result<ToolResult> {
        let prompt = input["prompt"].as_str().unwrap_or_default();
        let schedule_type = input["schedule_type"].as_str().unwrap_or_default();
        let schedule_value = input["schedule_value"].as_str().unwrap_or_default();
        let context_mode = input["context_mode"].as_str().unwrap_or("group");

        let target_module = input["target_module_id"]
            .as_str()
            .filter(|_| ctx.is_main)
            .unwrap_or(&ctx.addr.module_id);
        let target_channel = input["target_channel_id"]
            .as_str()
            .filter(|_| ctx.is_main)
            .unwrap_or(&ctx.addr.channel_id);

        if prompt.is_empty() || schedule_type.is_empty() || schedule_value.is_empty() {
            return Ok(ToolResult::error(
                "prompt, schedule_type, and schedule_value are required".into(),
            ));
        }

        ctx.ipc_sender
            .send(IpcCommand::ScheduleTask {
                prompt: prompt.to_string(),
                schedule_type: schedule_type.to_string(),
                schedule_value: schedule_value.to_string(),
                context_mode: context_mode.to_string(),
                target_addr: ChannelAddr::new(target_module, target_channel),
                source_workspace: ctx.workspace.clone(),
            })
            .await
            .map_err(|e| crate::error::MiraError::Tool(format!("Failed to send IPC: {}", e)))?;

        Ok(ToolResult::ok(format!(
            "Task scheduled: {} - {}",
            schedule_type, schedule_value
        )))
    }
}

// --- list_tasks ---
pub struct ListTasksTool;

#[async_trait]
impl Tool for ListTasksTool {
    fn name(&self) -> &str {
        "list_tasks"
    }

    fn description(&self) -> &str {
        "List all scheduled tasks. Main group sees all; others see only their own."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _input: Value, ctx: &ToolContext) -> crate::error::Result<ToolResult> {
        let tasks = if ctx.is_main {
            ctx.db.get_all_tasks()?
        } else {
            ctx.db.get_tasks_for_workspace(&ctx.workspace)?
        };

        if tasks.is_empty() {
            return Ok(ToolResult::ok("No scheduled tasks found.".into()));
        }

        let formatted: Vec<String> = tasks
            .iter()
            .map(|t| {
                format!(
                    "- [{}] {}... ({}: {}) - {}, next: {}",
                    t.id,
                    {
                        let s: String = t.prompt.chars().take(50).collect();
                        s
                    },
                    t.schedule_type.as_str(),
                    t.schedule_value,
                    t.status.as_str(),
                    t.next_run.as_deref().unwrap_or("N/A")
                )
            })
            .collect();

        Ok(ToolResult::ok(format!(
            "Scheduled tasks:\n{}",
            formatted.join("\n")
        )))
    }
}

// --- pause_task ---
pub struct PauseTaskTool;

#[async_trait]
impl Tool for PauseTaskTool {
    fn name(&self) -> &str {
        "pause_task"
    }

    fn description(&self) -> &str {
        "Pause a scheduled task."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task ID to pause"
                }
            },
            "required": ["task_id"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::error::Result<ToolResult> {
        let task_id = input["task_id"].as_str().unwrap_or_default();
        if task_id.is_empty() {
            return Ok(ToolResult::error("task_id is required".into()));
        }

        ctx.ipc_sender
            .send(IpcCommand::PauseTask {
                task_id: task_id.to_string(),
                source_workspace: ctx.workspace.clone(),
            })
            .await
            .map_err(|e| crate::error::MiraError::Tool(format!("Failed to send IPC: {}", e)))?;

        Ok(ToolResult::ok(format!("Task {} pause requested.", task_id)))
    }
}

// --- resume_task ---
pub struct ResumeTaskTool;

#[async_trait]
impl Tool for ResumeTaskTool {
    fn name(&self) -> &str {
        "resume_task"
    }

    fn description(&self) -> &str {
        "Resume a paused task."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task ID to resume"
                }
            },
            "required": ["task_id"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::error::Result<ToolResult> {
        let task_id = input["task_id"].as_str().unwrap_or_default();
        if task_id.is_empty() {
            return Ok(ToolResult::error("task_id is required".into()));
        }

        ctx.ipc_sender
            .send(IpcCommand::ResumeTask {
                task_id: task_id.to_string(),
                source_workspace: ctx.workspace.clone(),
            })
            .await
            .map_err(|e| crate::error::MiraError::Tool(format!("Failed to send IPC: {}", e)))?;

        Ok(ToolResult::ok(format!(
            "Task {} resume requested.",
            task_id
        )))
    }
}

// --- cancel_task ---
pub struct CancelTaskTool;

#[async_trait]
impl Tool for CancelTaskTool {
    fn name(&self) -> &str {
        "cancel_task"
    }

    fn description(&self) -> &str {
        "Cancel and delete a scheduled task."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task ID to cancel"
                }
            },
            "required": ["task_id"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::error::Result<ToolResult> {
        let task_id = input["task_id"].as_str().unwrap_or_default();
        if task_id.is_empty() {
            return Ok(ToolResult::error("task_id is required".into()));
        }

        ctx.ipc_sender
            .send(IpcCommand::CancelTask {
                task_id: task_id.to_string(),
                source_workspace: ctx.workspace.clone(),
            })
            .await
            .map_err(|e| crate::error::MiraError::Tool(format!("Failed to send IPC: {}", e)))?;

        Ok(ToolResult::ok(format!(
            "Task {} cancellation requested.",
            task_id
        )))
    }
}

// --- module_tool ---
pub struct ModuleToolCallTool;

#[async_trait]
impl Tool for ModuleToolCallTool {
    fn name(&self) -> &str {
        "module_tool"
    }

    fn description(&self) -> &str {
        "Call a module-specific tool."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "module_id": { "type": "string" },
                "tool_name": { "type": "string" },
                "input": { "type": "object" }
            },
            "required": ["module_id", "tool_name"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::error::Result<ToolResult> {
        let module_id = input["module_id"].as_str().unwrap_or_default();
        let tool_name = input["tool_name"].as_str().unwrap_or_default();
        let tool_input = input.get("input").cloned().unwrap_or_else(|| json!({}));

        if module_id.is_empty() || tool_name.is_empty() {
            return Ok(ToolResult::error(
                "module_id and tool_name are required".into(),
            ));
        }

        ctx.ipc_sender
            .send(IpcCommand::CallModuleTool {
                module_id: module_id.to_string(),
                tool_name: tool_name.to_string(),
                input: tool_input,
            })
            .await
            .map_err(|e| crate::error::MiraError::Tool(format!("Failed to send IPC: {}", e)))?;

        Ok(ToolResult::ok(format!(
            "Module tool request queued: {}.{}",
            module_id, tool_name
        )))
    }
}
