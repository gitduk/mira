use std::io::IsTerminal;
use std::path::PathBuf;
use regex::Regex;

pub struct Config {
    pub assistant_name: String,
    pub owner_name: String,
    pub poll_interval_ms: u64,
    pub scheduler_poll_interval_ms: u64,
    pub store_dir: PathBuf,
    pub groups_dir: PathBuf,
    pub data_dir: PathBuf,
    pub main_group_folder: String,
    pub max_concurrent_agents: usize,
    pub trigger_pattern: Regex,
    pub timezone: String,
    pub dashboard_enabled: bool,
    pub claude_model: String,
    pub log_level: String,
    pub api_base_url: String,
    pub api_key: String,
}

impl Config {
    pub fn from_env() -> Self {
        let assistant_name = std::env::var("ASSISTANT_NAME").unwrap_or_else(|_| "Mira".into());
        let owner_name = std::env::var("OWNER_NAME").unwrap_or_else(|_| "Owner".into());
        let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        let max_concurrent_agents = std::env::var("MAX_CONCURRENT_AGENTS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(5)
            .max(1);

        let escaped_name = regex::escape(&assistant_name);
        let trigger_pattern = Regex::new(&format!("(?i)^@{}\\b", escaped_name))
            .expect("Invalid trigger pattern");

        let timezone = std::env::var("TZ").unwrap_or_else(|_| "UTC".into());

        let dashboard_enabled = std::io::stdout().is_terminal()
            && std::env::var("DASHBOARD")
                .map(|v| v != "false")
                .unwrap_or(true);

        let claude_model = std::env::var("CLAUDE_MODEL")
            .unwrap_or_else(|_| "claude-sonnet-4.5".into());

        let log_level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".into());

        let api_base_url = std::env::var("ANTHROPIC_BASE_URL")
            .unwrap_or_else(|_| "https://api.anthropic.com".into());
        let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| String::new());

        Config {
            assistant_name,
            owner_name,
            poll_interval_ms: 2000,
            scheduler_poll_interval_ms: 60000,
            store_dir: project_root.join("store"),
            groups_dir: project_root.join("groups"),
            data_dir: project_root.join("data"),
            main_group_folder: "main".into(),
            max_concurrent_agents,
            trigger_pattern,
            timezone,
            dashboard_enabled,
            claude_model,
            log_level,
            api_base_url,
            api_key,
        }
    }

    pub fn is_main_group(&self, folder: &str) -> bool {
        folder == self.main_group_folder
    }
}
