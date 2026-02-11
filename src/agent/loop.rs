use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::api::{AnthropicClient, ContentBlock, Delta, Message, MessagesRequest, StreamEvent};
use super::session::Session;
use super::tools::{ToolContext, ToolRegistry, ToolResult};
use super::{AgentResponse, OnProgressCallback, OutputType};
use crate::error::{MiraError, Result};

const MAX_TURNS: usize = 50;

#[derive(Debug, Clone)]
pub struct AgentProgressEvent {
    pub event_type: ProgressEventType,
    pub text: String,
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ProgressEventType {
    Text,
    TextStreaming,
    ToolUse,
    ToolSummary,
}

pub struct LoopResult {
    pub response: AgentResponse,
}

#[allow(clippy::too_many_arguments)]
pub async fn agentic_loop(
    api_base_url: &str,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    initial_prompt: &str,
    session: &Session,
    tool_ctx: ToolContext,
    on_progress: Option<OnProgressCallback>,
) -> Result<LoopResult> {
    let api_base_url = api_base_url.to_string();
    let api_key = api_key.to_string();
    let registry = ToolRegistry::new(tool_ctx);
    let tool_defs = registry.tool_definitions();

    // Build initial messages from session history + new prompt
    let mut messages: Vec<Message> = session.messages.clone();
    messages.push(Message {
        role: "user".into(),
        content: vec![ContentBlock::Text {
            text: initial_prompt.to_string(),
        }],
    });

    let mut last_text = String::new();

    for turn in 0..MAX_TURNS {
        debug!(turn, "Starting agentic turn");

        let mut request = MessagesRequest {
            model: model.to_string(),
            max_tokens: 8192,
            system: system_prompt.to_string(),
            messages: messages.clone(),
            tools: tool_defs.clone(),
            stream: None,
        };

        // Set up streaming channel
        let (event_tx, mut event_rx) = mpsc::channel::<StreamEvent>(100);

        // Progress tracking state
        let mut current_tool_name = String::new();
        let mut current_tool_input = String::new();
        let mut text_buffer = String::new();

        // Create a new client each iteration (stateless, avoids move issues)
        let client = AnthropicClient::new(&api_base_url, &api_key);

        // Spawn API call
        let api_handle =
            tokio::spawn(async move { client.stream_messages(&mut request, event_tx).await });

        // Process stream events for progress
        while let Some(event) = event_rx.recv().await {
            if let Some(ref cb) = on_progress {
                match &event {
                    StreamEvent::ContentBlockStart {
                        content_block: ContentBlock::ToolUse { name, .. },
                        ..
                    } => {
                        current_tool_name = name.clone();
                        current_tool_input.clear();
                    }
                    StreamEvent::ContentBlockDelta { delta, .. } => match delta {
                        Delta::TextDelta { text } => {
                            text_buffer.push_str(text);
                            // Flush complete lines
                            while let Some(pos) = text_buffer.find('\n') {
                                let line = text_buffer[..pos].trim().to_string();
                                text_buffer = text_buffer[pos + 1..].to_string();
                                if !line.is_empty() {
                                    cb(AgentProgressEvent {
                                        event_type: ProgressEventType::Text,
                                        text: line,
                                        tool_name: None,
                                    });
                                }
                            }
                            // Stream incomplete line
                            if !text_buffer.trim().is_empty() {
                                cb(AgentProgressEvent {
                                    event_type: ProgressEventType::TextStreaming,
                                    text: text_buffer.clone(),
                                    tool_name: None,
                                });
                            }
                        }
                        Delta::InputJsonDelta { partial_json } => {
                            current_tool_input.push_str(partial_json);
                        }
                    },
                    StreamEvent::ContentBlockStop { .. } => {
                        // Flush remaining text
                        if !text_buffer.trim().is_empty() {
                            cb(AgentProgressEvent {
                                event_type: ProgressEventType::Text,
                                text: text_buffer.trim().to_string(),
                                tool_name: None,
                            });
                        }
                        text_buffer.clear();

                        // Emit tool summary
                        if !current_tool_name.is_empty() {
                            let summary =
                                format_tool_summary(&current_tool_name, &current_tool_input);
                            cb(AgentProgressEvent {
                                event_type: ProgressEventType::ToolUse,
                                text: summary,
                                tool_name: Some(current_tool_name.clone()),
                            });
                            current_tool_name.clear();
                            current_tool_input.clear();
                        }
                    }
                    _ => {}
                }
            }
        }

        // Wait for API response
        let response = api_handle
            .await
            .map_err(|e| MiraError::Agent(format!("API task failed: {}", e)))??;

        // Add assistant response to messages
        messages.push(Message {
            role: "assistant".into(),
            content: response.content.clone(),
        });

        // Extract text from response
        for block in &response.content {
            if let ContentBlock::Text { text } = block {
                if !text.trim().is_empty() {
                    last_text = text.clone();
                }
            }
        }

        // Check stop reason
        match response.stop_reason.as_deref() {
            Some("end_turn") | Some("stop") | None => {
                // Agent is done
                let response = if last_text.trim().is_empty() {
                    AgentResponse {
                        output_type: OutputType::Log,
                        user_message: None,
                        internal_log: None,
                    }
                } else {
                    AgentResponse {
                        output_type: OutputType::Message,
                        user_message: Some(last_text.clone()),
                        internal_log: None,
                    }
                };

                return Ok(LoopResult { response });
            }
            Some("tool_use") => {
                // Execute tool calls
                let mut tool_results = Vec::new();

                for block in &response.content {
                    if let ContentBlock::ToolUse { id, name, input } = block {
                        info!(tool = %name, "Executing tool");

                        let result = registry.execute(name, input.clone()).await;

                        let (content, is_error) = match result {
                            Ok(ToolResult { content, is_error }) => {
                                if let Some(ref cb) = on_progress {
                                    let summary = if content.len() > 200 {
                                        let end = content.floor_char_boundary(200);
                                        format!("{}...", &content[..end])
                                    } else {
                                        content.clone()
                                    };
                                    cb(AgentProgressEvent {
                                        event_type: ProgressEventType::ToolSummary,
                                        text: summary,
                                        tool_name: Some(name.clone()),
                                    });
                                }
                                (content, is_error)
                            }
                            Err(e) => {
                                let error_msg = format!("Tool execution error: {}", e);
                                error!(tool = %name, %error_msg);
                                (error_msg, true)
                            }
                        };

                        tool_results.push(ContentBlock::ToolResult {
                            tool_use_id: id.clone(),
                            content,
                            is_error: if is_error { Some(true) } else { None },
                        });
                    }
                }

                // Add tool results to messages
                messages.push(Message {
                    role: "user".into(),
                    content: tool_results,
                });
            }
            Some(reason) => {
                warn!("Unexpected stop reason: {}", reason);
                return Ok(LoopResult {
                    response: AgentResponse {
                        output_type: OutputType::Log,
                        user_message: None,
                        internal_log: Some(format!("Stopped: {}", reason)),
                    },
                });
            }
        }

        // Client is recreated at the top of each iteration
    }

    Err(MiraError::Agent(format!(
        "Exceeded maximum {} agentic turns",
        MAX_TURNS
    )))
}

fn format_tool_summary(tool_name: &str, input_json: &str) -> String {
    let input: Value =
        serde_json::from_str(input_json).unwrap_or(Value::Object(Default::default()));

    match tool_name {
        "Read" => {
            let file_path = input["file_path"].as_str().unwrap_or("?");
            format!("Read {}", file_path)
        }
        "Bash" => {
            let cmd = input["command"].as_str().unwrap_or("?");
            let short = cmd.lines().next().unwrap_or(cmd);
            if short.len() > 60 {
                format!("$ {}...", &short[..60])
            } else {
                format!("$ {}", short)
            }
        }
        "Grep" => {
            let pattern = input["pattern"].as_str().unwrap_or("?");
            let path = input["path"].as_str().unwrap_or("");
            if path.is_empty() {
                format!("Grep \"{}\"", pattern)
            } else {
                format!("Grep \"{}\" in {}", pattern, path)
            }
        }
        "Glob" => {
            let pattern = input["pattern"].as_str().unwrap_or("?");
            format!("Glob {}", pattern)
        }
        "Edit" => {
            let file_path = input["file_path"].as_str().unwrap_or("?");
            format!("Edit {}", file_path)
        }
        "Write" => {
            let file_path = input["file_path"].as_str().unwrap_or("?");
            format!("Write {}", file_path)
        }
        "WebSearch" => {
            let query = input["query"].as_str().unwrap_or("?");
            format!("Search: {}", query)
        }
        "WebFetch" => {
            let url = input["url"].as_str().unwrap_or("?");
            format!("Fetch: {}", url)
        }
        _ => {
            if tool_name.starts_with("mcp__mira__") {
                tool_name.replace("mcp__mira__", "")
            } else {
                tool_name.to_string()
            }
        }
    }
}
