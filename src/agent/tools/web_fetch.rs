use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolContext, ToolResult};

const MAX_CONTENT_LEN: usize = 50_000;

pub struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "WebFetch"
    }

    fn description(&self) -> &str {
        "Fetch content from a URL. Returns the page content as text."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch"
                },
                "prompt": {
                    "type": "string",
                    "description": "What information to extract from the page"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> crate::error::Result<ToolResult> {
        let url = input["url"].as_str().unwrap_or_default();

        if url.is_empty() {
            return Ok(ToolResult::error("url is required".into()));
        }

        // Upgrade HTTP to HTTPS
        let url = if url.starts_with("http://") {
            url.replacen("http://", "https://", 1)
        } else {
            url.to_string()
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(crate::error::MiraError::Http)?;

        match client
            .get(&url)
            .header("User-Agent", "Mira/1.0")
            .send()
            .await
        {
            Ok(response) => {
                if !response.status().is_success() {
                    return Ok(ToolResult::error(format!(
                        "HTTP {} for {}",
                        response.status(),
                        url
                    )));
                }

                match response.text().await {
                    Ok(body) => {
                        // Convert HTML to plain text
                        let text = html2text::from_read(body.as_bytes(), 80);

                        let truncated = if text.len() > MAX_CONTENT_LEN {
                            format!(
                                "{}...\n\n(content truncated, {} chars total)",
                                &text[..MAX_CONTENT_LEN],
                                text.len()
                            )
                        } else {
                            text
                        };

                        Ok(ToolResult::ok(truncated))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to read response: {}", e))),
                }
            }
            Err(e) => Ok(ToolResult::error(format!("Request failed: {}", e))),
        }
    }
}
