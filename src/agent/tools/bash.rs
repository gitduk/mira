use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;
use tracing::debug;

use super::{Tool, ToolContext, ToolResult};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 600_000;
const MAX_OUTPUT_LEN: usize = 30_000;

pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn description(&self) -> &str {
        "Execute a bash command. The working directory is the group's workspace."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout": {
                    "type": "number",
                    "description": "Optional timeout in milliseconds (max 600000)"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::error::Result<ToolResult> {
        let command = input["command"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        if command.is_empty() {
            return Ok(ToolResult::error("Command is required".into()));
        }

        let timeout_ms = input["timeout"]
            .as_u64()
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);

        debug!(command = %command, timeout_ms, "Executing bash command");

        let timeout = tokio::time::Duration::from_millis(timeout_ms);

        let result = tokio::time::timeout(timeout, async {
            Command::new("bash")
                .arg("-c")
                .arg(&command)
                .current_dir(&ctx.workspace_dir)
                .output()
                .await
        })
        .await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                let mut result_text = String::new();

                if !stdout.is_empty() {
                    let truncated = truncate_output(&stdout, MAX_OUTPUT_LEN);
                    result_text.push_str(&truncated);
                }
                if !stderr.is_empty() {
                    if !result_text.is_empty() {
                        result_text.push('\n');
                    }
                    result_text.push_str("STDERR:\n");
                    let truncated = truncate_output(&stderr, MAX_OUTPUT_LEN / 2);
                    result_text.push_str(&truncated);
                }

                if result_text.is_empty() {
                    result_text = format!("(exit code: {})", output.status.code().unwrap_or(-1));
                }

                let is_error = !output.status.success();
                Ok(ToolResult {
                    content: result_text,
                    is_error,
                })
            }
            Ok(Err(e)) => Ok(ToolResult::error(format!("Failed to execute: {}", e))),
            Err(_) => Ok(ToolResult::error(format!(
                "Command timed out after {}ms",
                timeout_ms
            ))),
        }
    }
}

fn truncate_output(output: &str, max_len: usize) -> String {
    if output.len() <= max_len {
        output.to_string()
    } else {
        format!(
            "{}...\n\n(output truncated, {} chars total)",
            &output[..max_len],
            output.len()
        )
    }
}
