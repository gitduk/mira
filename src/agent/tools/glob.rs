use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

use super::{Tool, ToolContext, ToolResult};

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern. Returns matching file paths."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match (e.g. '**/*.rs', 'src/**/*.ts')"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (defaults to workspace)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::error::Result<ToolResult> {
        let pattern = input["pattern"]
            .as_str()
            .unwrap_or_default();
        let search_path = input["path"]
            .as_str()
            .map(|p| {
                let pb = PathBuf::from(p);
                if pb.is_absolute() {
                    pb
                } else {
                    ctx.workspace_dir.join(pb)
                }
            })
            .unwrap_or_else(|| ctx.workspace_dir.clone());

        if pattern.is_empty() {
            return Ok(ToolResult::error("pattern is required".into()));
        }

        match globwalk::GlobWalkerBuilder::from_patterns(&search_path, &[pattern])
            .max_depth(20)
            .follow_links(false)
            .build()
        {
            Ok(walker) => {
                let mut paths: Vec<String> = walker
                    .filter_map(|e| e.ok())
                    .map(|e| e.path().display().to_string())
                    .collect();

                paths.sort();

                if paths.is_empty() {
                    Ok(ToolResult::ok("No files found matching pattern.".into()))
                } else {
                    let count = paths.len();
                    let result = if count > 500 {
                        let truncated = paths[..500].join("\n");
                        format!(
                            "{}\n\n... and {} more files (showing first 500)",
                            truncated,
                            count - 500
                        )
                    } else {
                        paths.join("\n")
                    };
                    Ok(ToolResult::ok(result))
                }
            }
            Err(e) => Ok(ToolResult::error(format!("Invalid glob pattern: {}", e))),
        }
    }
}
