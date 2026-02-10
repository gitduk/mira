use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::path::PathBuf;

use super::{Tool, ToolContext, ToolResult};

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn description(&self) -> &str {
        "Search file contents using regex patterns. Supports filtering by file type and glob."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regular expression pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search in"
                },
                "glob": {
                    "type": "string",
                    "description": "Glob pattern to filter files (e.g. '*.rs')"
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"],
                    "description": "Output mode (default: files_with_matches)"
                },
                "context": {
                    "type": "number",
                    "description": "Lines of context around matches"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::error::Result<ToolResult> {
        let pattern_str = input["pattern"].as_str().unwrap_or_default();
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
        let glob_pattern = input["glob"].as_str();
        let output_mode = input["output_mode"]
            .as_str()
            .unwrap_or("files_with_matches");
        let context_lines = input["context"].as_u64().unwrap_or(0) as usize;

        if pattern_str.is_empty() {
            return Ok(ToolResult::error("pattern is required".into()));
        }

        let regex = match Regex::new(pattern_str) {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::error(format!("Invalid regex: {}", e))),
        };

        // Collect files to search
        let files = if search_path.is_file() {
            vec![search_path.clone()]
        } else {
            let glob = glob_pattern.unwrap_or("**/*");
            match globwalk::GlobWalkerBuilder::from_patterns(&search_path, &[glob])
                .max_depth(20)
                .follow_links(false)
                .file_type(globwalk::FileType::FILE)
                .build()
            {
                Ok(walker) => walker
                    .filter_map(|e| e.ok())
                    .map(|e| e.path().to_path_buf())
                    .collect(),
                Err(e) => return Ok(ToolResult::error(format!("Glob error: {}", e))),
            }
        };

        let mut result = String::new();
        let mut _match_count = 0;

        for file in &files {
            let content = match std::fs::read_to_string(file) {
                Ok(c) => c,
                Err(_) => continue, // Skip binary/unreadable files
            };

            let lines: Vec<&str> = content.lines().collect();
            let mut file_matches = Vec::new();

            for (i, line) in lines.iter().enumerate() {
                if regex.is_match(line) {
                    file_matches.push(i);
                    _match_count += 1;
                }
            }

            if file_matches.is_empty() {
                continue;
            }

            match output_mode {
                "files_with_matches" => {
                    result.push_str(&format!("{}\n", file.display()));
                }
                "count" => {
                    result.push_str(&format!(
                        "{}:{}\n",
                        file.display(),
                        file_matches.len()
                    ));
                }
                "content" | _ => {
                    for &line_idx in &file_matches {
                        let start = line_idx.saturating_sub(context_lines);
                        let end = (line_idx + context_lines + 1).min(lines.len());

                        for j in start..end {
                            let prefix = if j == line_idx { ">" } else { " " };
                            result.push_str(&format!(
                                "{}:{}:{} {}\n",
                                file.display(),
                                j + 1,
                                prefix,
                                lines[j]
                            ));
                        }
                        if context_lines > 0 {
                            result.push_str("--\n");
                        }
                    }
                }
            }

            // Limit output
            if result.len() > 50_000 {
                result.push_str("\n... (output truncated)\n");
                break;
            }
        }

        if result.is_empty() {
            Ok(ToolResult::ok("No matches found.".into()))
        } else {
            Ok(ToolResult::ok(result))
        }
    }
}
