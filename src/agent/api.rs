use std::time::Duration;

use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::warn;

use crate::error::{MiraError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct MessagesRequest {
    pub model: String,
    pub max_tokens: u32,
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessagesResponse {
    pub id: String,
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// SSE event from streaming API
#[derive(Debug, Clone)]
pub enum StreamEvent {
    ContentBlockStart {
        index: usize,
        content_block: ContentBlock,
    },
    ContentBlockDelta {
        index: usize,
        delta: Delta,
    },
    ContentBlockStop {
        index: usize,
    },
    MessageStart {
        message_id: String,
    },
    MessageDelta {
        stop_reason: Option<String>,
    },
    MessageStop,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum Delta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
}

pub struct AnthropicClient {
    client: Client,
    base_url: String,
    api_key: String,
}

impl AnthropicClient {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("Failed to build HTTP client");

        AnthropicClient {
            client,
            base_url: base_url.to_string(),
            api_key: api_key.to_string(),
        }
    }

    /// Call Messages API with streaming, returning events via channel
    pub async fn stream_messages(
        &self,
        request: &mut MessagesRequest,
        event_tx: mpsc::Sender<StreamEvent>,
    ) -> Result<MessagesResponse> {
        request.stream = Some(true);

        let mut retries = 0;
        let max_retries = 3;

        loop {
            match self.do_stream_request(request, &event_tx).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    retries += 1;
                    if retries > max_retries {
                        return Err(e);
                    }

                    // Only retry on specific errors
                    let should_retry = match &e {
                        MiraError::Http(req_err) => {
                            req_err.status().is_some_and(|s| {
                                s.as_u16() == 429 || s.as_u16() >= 500
                            })
                        }
                        _ => false,
                    };

                    if !should_retry {
                        return Err(e);
                    }

                    let delay = Duration::from_secs(2u64.pow(retries));
                    warn!("API request failed (retry {}/{}), waiting {:?}: {}", retries, max_retries, delay, e);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    async fn do_stream_request(
        &self,
        request: &MessagesRequest,
        event_tx: &mpsc::Sender<StreamEvent>,
    ) -> Result<MessagesResponse> {
        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("Content-Type", "application/json")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(request)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(MiraError::Agent(format!(
                "API returned status {}: {}",
                status, body
            )));
        }

        // Parse SSE stream
        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        let mut current_text = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_id = String::new();
        let mut current_tool_input = String::new();
        let mut stop_reason = None;
        let mut message_id = String::new();
        let mut current_block_type: Option<String> = None;

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete SSE lines
            while let Some(pos) = buffer.find("\n\n") {
                let event_str = buffer[..pos].to_string();
                buffer = buffer[pos + 2..].to_string();

                // Parse SSE event
                let mut event_type = String::new();
                let mut event_data = String::new();

                for line in event_str.lines() {
                    if let Some(t) = line.strip_prefix("event: ") {
                        event_type = t.to_string();
                    } else if let Some(d) = line.strip_prefix("data: ") {
                        event_data = d.to_string();
                    }
                }

                if event_data.is_empty() {
                    continue;
                }

                match event_type.as_str() {
                    "message_start" => {
                        if let Ok(data) = serde_json::from_str::<Value>(&event_data) {
                            message_id = data["message"]["id"]
                                .as_str()
                                .unwrap_or_default()
                                .to_string();
                            let _ = event_tx
                                .send(StreamEvent::MessageStart {
                                    message_id: message_id.clone(),
                                })
                                .await;
                        }
                    }
                    "content_block_start" => {
                        if let Ok(data) = serde_json::from_str::<Value>(&event_data) {
                            let index = data["index"].as_u64().unwrap_or(0) as usize;
                            let block = &data["content_block"];
                            let block_type = block["type"].as_str().unwrap_or("text");

                            current_block_type = Some(block_type.to_string());

                            if block_type == "tool_use" {
                                current_tool_name = block["name"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .to_string();
                                current_tool_id = block["id"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .to_string();
                                current_tool_input.clear();

                                let _ = event_tx
                                    .send(StreamEvent::ContentBlockStart {
                                        index,
                                        content_block: ContentBlock::ToolUse {
                                            id: current_tool_id.clone(),
                                            name: current_tool_name.clone(),
                                            input: Value::Null,
                                        },
                                    })
                                    .await;
                            } else {
                                current_text.clear();
                                let _ = event_tx
                                    .send(StreamEvent::ContentBlockStart {
                                        index,
                                        content_block: ContentBlock::Text {
                                            text: String::new(),
                                        },
                                    })
                                    .await;
                            }
                        }
                    }
                    "content_block_delta" => {
                        if let Ok(data) = serde_json::from_str::<Value>(&event_data) {
                            let index = data["index"].as_u64().unwrap_or(0) as usize;
                            let delta = &data["delta"];
                            let delta_type = delta["type"].as_str().unwrap_or("");

                            if delta_type == "text_delta" {
                                let text = delta["text"].as_str().unwrap_or_default();
                                current_text.push_str(text);
                                let _ = event_tx
                                    .send(StreamEvent::ContentBlockDelta {
                                        index,
                                        delta: Delta::TextDelta {
                                            text: text.to_string(),
                                        },
                                    })
                                    .await;
                            } else if delta_type == "input_json_delta" {
                                let json = delta["partial_json"]
                                    .as_str()
                                    .unwrap_or_default();
                                current_tool_input.push_str(json);
                                let _ = event_tx
                                    .send(StreamEvent::ContentBlockDelta {
                                        index,
                                        delta: Delta::InputJsonDelta {
                                            partial_json: json.to_string(),
                                        },
                                    })
                                    .await;
                            }
                        }
                    }
                    "content_block_stop" => {
                        if let Ok(data) = serde_json::from_str::<Value>(&event_data) {
                            let index = data["index"].as_u64().unwrap_or(0) as usize;

                            match current_block_type.as_deref() {
                                Some("tool_use") => {
                                    let input: Value =
                                        serde_json::from_str(&current_tool_input)
                                            .unwrap_or(Value::Object(Default::default()));
                                    content_blocks.push(ContentBlock::ToolUse {
                                        id: current_tool_id.clone(),
                                        name: current_tool_name.clone(),
                                        input,
                                    });
                                }
                                _ => {
                                    if !current_text.is_empty() {
                                        content_blocks.push(ContentBlock::Text {
                                            text: current_text.clone(),
                                        });
                                    }
                                }
                            }

                            current_block_type = None;
                            let _ = event_tx
                                .send(StreamEvent::ContentBlockStop { index })
                                .await;
                        }
                    }
                    "message_delta" => {
                        if let Ok(data) = serde_json::from_str::<Value>(&event_data) {
                            stop_reason = data["delta"]["stop_reason"]
                                .as_str()
                                .map(|s| s.to_string());
                            let _ = event_tx
                                .send(StreamEvent::MessageDelta {
                                    stop_reason: stop_reason.clone(),
                                })
                                .await;
                        }
                    }
                    "message_stop" => {
                        let _ = event_tx.send(StreamEvent::MessageStop).await;
                    }
                    _ => {}
                }
            }
        }

        Ok(MessagesResponse {
            id: message_id,
            content: content_blocks,
            stop_reason,
            usage: None,
        })
    }

    /// Non-streaming call for simpler use cases
    pub async fn call_messages(&self, request: &MessagesRequest) -> Result<MessagesResponse> {
        let mut retries = 0;
        let max_retries = 3;

        loop {
            let response = self
                .client
                .post(format!("{}/v1/messages", self.base_url))
                .header("Content-Type", "application/json")
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(request)
                .send()
                .await?;

            let status = response.status();
            if status.is_success() {
                return Ok(response.json().await?);
            }

            retries += 1;
            if retries > max_retries || !(status.as_u16() == 429 || status.as_u16() >= 500) {
                let body = response.text().await.unwrap_or_default();
                return Err(MiraError::Agent(format!(
                    "API returned status {}: {}",
                    status, body
                )));
            }

            let delay = Duration::from_secs(2u64.pow(retries));
            warn!("API request failed (retry {}/{}), waiting {:?}", retries, max_retries, delay);
            tokio::time::sleep(delay).await;
        }
    }
}
