use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

use super::{Tool, ToolContext, ToolResult};

const DEFAULT_LIMIT: usize = 2000;
const MAX_LINE_LEN: usize = 2000;

pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "Read"
    }

    fn description(&self) -> &str {
        "Read a file from the filesystem. Returns content with line numbers."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The path to the file to read"
                },
                "offset": {
                    "type": "number",
                    "description": "Line number to start reading from (1-based)"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of lines to read"
                }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::error::Result<ToolResult> {
        let file_path = input["file_path"]
            .as_str()
            .unwrap_or_default();

        if file_path.is_empty() {
            return Ok(ToolResult::error("file_path is required".into()));
        }

        let path = resolve_path(file_path, &ctx.workspace_dir);

        if !path.exists() {
            return Ok(ToolResult::error(format!(
                "File not found: {}",
                path.display()
            )));
        }

        if path.is_dir() {
            return Ok(ToolResult::error(
                "Cannot read a directory. Use Bash with 'ls' to list directory contents.".into(),
            ));
        }

        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let limit = input["limit"].as_u64().unwrap_or(DEFAULT_LIMIT as u64) as usize;

        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let start = if offset > 0 { offset - 1 } else { 0 };
                let end = (start + limit).min(lines.len());

                let mut result = String::new();
                for (i, line) in lines[start..end].iter().enumerate() {
                    let line_num = start + i + 1;
                    let truncated = if line.len() > MAX_LINE_LEN {
                        format!("{}...", &line[..MAX_LINE_LEN])
                    } else {
                        line.to_string()
                    };
                    result.push_str(&format!("{:>6}\t{}\n", line_num, truncated));
                }

                if result.is_empty() {
                    result = "(empty file)".into();
                }

                Ok(ToolResult::ok(result))
            }
            Err(e) => Ok(ToolResult::error(format!("Failed to read file: {}", e))),
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
