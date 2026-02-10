use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolContext, ToolResult};

pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "WebSearch"
    }

    fn description(&self) -> &str {
        "Search the web and return results. Use for current information beyond training data."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> crate::error::Result<ToolResult> {
        let query = input["query"].as_str().unwrap_or_default();

        if query.is_empty() {
            return Ok(ToolResult::error("query is required".into()));
        }

        // Use a search API - for now use a simple HTTP-based approach
        // In production, this would use Brave Search API, Google Custom Search, etc.
        let search_api_key = std::env::var("SEARCH_API_KEY").ok();
        let search_api_url = std::env::var("SEARCH_API_URL").ok();

        if let (Some(api_key), Some(api_url)) = (search_api_key, search_api_url) {
            let client = reqwest::Client::new();
            match client
                .get(&api_url)
                .query(&[("q", query), ("key", &api_key)])
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        match response.text().await {
                            Ok(body) => Ok(ToolResult::ok(body)),
                            Err(e) => Ok(ToolResult::error(format!("Failed to read response: {}", e))),
                        }
                    } else {
                        Ok(ToolResult::error(format!(
                            "Search API returned status: {}",
                            response.status()
                        )))
                    }
                }
                Err(e) => Ok(ToolResult::error(format!("Search request failed: {}", e))),
            }
        } else {
            Ok(ToolResult::error(
                "Web search not configured. Set SEARCH_API_KEY and SEARCH_API_URL environment variables.".into(),
            ))
        }
    }
}
