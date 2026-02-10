use thiserror::Error;

pub type Result<T> = std::result::Result<T, MiraError>;

#[derive(Error, Debug)]
pub enum MiraError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Bridge error: {0}")]
    Bridge(String),

    #[error("Agent error: {0}")]
    Agent(String),

    #[error("Tool error: {0}")]
    Tool(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Config error: {0}")]
    Config(String),
}
