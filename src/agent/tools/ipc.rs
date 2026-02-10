use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolContext, ToolResult};
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

        ctx.ipc_sender
            .send(IpcCommand::SendMessage {
                chat_jid: ctx.chat_jid.clone(),
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
                    "description": "cron expression, milliseconds, or ISO timestamp"
                },
                "context_mode": {
                    "type": "string",
                    "enum": ["group", "isolated"],
                    "description": "group=with chat history, isolated=fresh session"
                },
                "target_group_jid": {
                    "type": "string",
                    "description": "JID of target group (main only, defaults to current)"
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
        let target_jid = input["target_group_jid"]
            .as_str()
            .filter(|_| ctx.is_main)
            .unwrap_or(&ctx.chat_jid);

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
                target_jid: target_jid.to_string(),
                source_group: ctx.group_folder.clone(),
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
            ctx.db.get_tasks_for_group(&ctx.group_folder)?
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
                    if t.prompt.len() > 50 {
                        &t.prompt[..50]
                    } else {
                        &t.prompt
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
                source_group: ctx.group_folder.clone(),
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
                source_group: ctx.group_folder.clone(),
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
                source_group: ctx.group_folder.clone(),
            })
            .await
            .map_err(|e| crate::error::MiraError::Tool(format!("Failed to send IPC: {}", e)))?;

        Ok(ToolResult::ok(format!(
            "Task {} cancellation requested.",
            task_id
        )))
    }
}

// --- register_group ---
pub struct RegisterGroupTool;

#[async_trait]
impl Tool for RegisterGroupTool {
    fn name(&self) -> &str {
        "register_group"
    }

    fn description(&self) -> &str {
        "Register a new WhatsApp group. Main group only."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "jid": {
                    "type": "string",
                    "description": "The WhatsApp JID"
                },
                "name": {
                    "type": "string",
                    "description": "Display name for the group"
                },
                "folder": {
                    "type": "string",
                    "description": "Folder name (lowercase, hyphens)"
                },
                "trigger": {
                    "type": "string",
                    "description": "Trigger word (e.g. '@Mira')"
                }
            },
            "required": ["jid", "name", "folder", "trigger"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::error::Result<ToolResult> {
        if !ctx.is_main {
            return Ok(ToolResult::error(
                "Only the main group can register new groups.".into(),
            ));
        }

        let jid = input["jid"].as_str().unwrap_or_default();
        let name = input["name"].as_str().unwrap_or_default();
        let folder = input["folder"].as_str().unwrap_or_default();
        let trigger = input["trigger"].as_str().unwrap_or_default();

        if jid.is_empty() || name.is_empty() || folder.is_empty() || trigger.is_empty() {
            return Ok(ToolResult::error(
                "jid, name, folder, and trigger are all required".into(),
            ));
        }

        ctx.ipc_sender
            .send(IpcCommand::RegisterGroup {
                jid: jid.to_string(),
                name: name.to_string(),
                folder: folder.to_string(),
                trigger: trigger.to_string(),
            })
            .await
            .map_err(|e| crate::error::MiraError::Tool(format!("Failed to send IPC: {}", e)))?;

        Ok(ToolResult::ok(format!(
            "Group \"{}\" registered. It will start receiving messages immediately.",
            name
        )))
    }
}

