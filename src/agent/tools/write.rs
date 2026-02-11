use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use super::{Tool, ToolContext, ToolResult};

pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "Write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates parent directories if needed."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::error::Result<ToolResult> {
        let file_path = input["file_path"].as_str().unwrap_or_default();
        let content = input["content"].as_str().unwrap_or_default();

        if file_path.is_empty() {
            return Ok(ToolResult::error("file_path is required".into()));
        }

        let path = resolve_path(file_path, &ctx.workspace_dir);

        // Create parent directories
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Ok(ToolResult::error(format!(
                    "Failed to create directories: {}",
                    e
                )));
            }
        }

        match std::fs::write(&path, content) {
            Ok(_) => Ok(ToolResult::ok(format!(
                "File written successfully: {} ({} bytes)",
                path.display(),
                content.len()
            ))),
            Err(e) => Ok(ToolResult::error(format!("Failed to write file: {}", e))),
        }
    }
}

fn resolve_path(file_path: &str, workspace_dir: &Path) -> PathBuf {
    let path = PathBuf::from(file_path);
    if path.is_absolute() {
        path
    } else {
        workspace_dir.join(path)
    }
}
