use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

use super::{Tool, ToolContext, ToolResult};

pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "Edit"
    }

    fn description(&self) -> &str {
        "Perform exact string replacement in a file. The old_string must be unique in the file."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact text to find and replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement text"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default false)"
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::error::Result<ToolResult> {
        let file_path = input["file_path"].as_str().unwrap_or_default();
        let old_string = input["old_string"].as_str().unwrap_or_default();
        let new_string = input["new_string"].as_str().unwrap_or_default();
        let replace_all = input["replace_all"].as_bool().unwrap_or(false);

        if file_path.is_empty() || old_string.is_empty() {
            return Ok(ToolResult::error(
                "file_path and old_string are required".into(),
            ));
        }

        if old_string == new_string {
            return Ok(ToolResult::error(
                "old_string and new_string must be different".into(),
            ));
        }

        let path = resolve_path(file_path, &ctx.workspace_dir);

        if !path.exists() {
            return Ok(ToolResult::error(format!(
                "File not found: {}",
                path.display()
            )));
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::error(format!("Failed to read file: {}", e))),
        };

        let count = content.matches(old_string).count();
        if count == 0 {
            return Ok(ToolResult::error(
                "old_string not found in file. Ensure the string matches exactly.".into(),
            ));
        }

        if !replace_all && count > 1 {
            return Ok(ToolResult::error(format!(
                "old_string found {} times. Provide more context to make it unique, or use replace_all.",
                count
            )));
        }

        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        match std::fs::write(&path, &new_content) {
            Ok(_) => {
                let msg = if replace_all && count > 1 {
                    format!("Replaced {} occurrences in {}", count, path.display())
                } else {
                    format!("Edit applied to {}", path.display())
                };
                Ok(ToolResult::ok(msg))
            }
            Err(e) => Ok(ToolResult::error(format!("Failed to write file: {}", e))),
        }
    }
}

fn resolve_path(file_path: &str, workspace_dir: &PathBuf) -> PathBuf {
    let path = PathBuf::from(file_path);
    if path.is_absolute() {
        path
    } else {
        workspace_dir.join(path)
    }
}
