use regex::Regex;
use std::io::IsTerminal;
use std::path::PathBuf;

pub struct Config {
    pub assistant_name: String,
    pub owner_name: String,
    pub poll_interval_ms: u64,
    pub scheduler_poll_interval_ms: u64,
    pub store_dir: PathBuf,
    pub groups_dir: PathBuf,
    pub data_dir: PathBuf,
    pub main_workspace: String,
    pub max_concurrent_agents: usize,
    pub trigger_pattern: Regex,
    pub timezone: String,
    pub dashboard_enabled: bool,
    pub dashboard_max_lines: usize,
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
        let trigger_pattern =
            Regex::new(&format!("(?i)^@{}\\b", escaped_name)).expect("Invalid trigger pattern");

        let timezone = std::env::var("TZ")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(detect_system_timezone)
            .unwrap_or_else(|| "UTC".into());

        let dashboard_enabled = std::io::stdout().is_terminal()
            && std::env::var("DASHBOARD")
                .map(|v| v != "false")
                .unwrap_or(true);
        let dashboard_max_lines = std::env::var("DASHBOARD_MAX_LINES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(500)
            .max(50);

        let claude_model =
            std::env::var("CLAUDE_MODEL").unwrap_or_else(|_| "claude-sonnet-4.5".into());

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
            main_workspace: "main".into(),
            max_concurrent_agents,
            trigger_pattern,
            timezone,
            dashboard_enabled,
            dashboard_max_lines,
            claude_model,
            log_level,
            api_base_url,
            api_key,
        }
    }

    pub fn is_main_workspace(&self, workspace: &str) -> bool {
        workspace == self.main_workspace
    }
}

/// Detect system timezone from /etc/localtime symlink or timedatectl.
fn detect_system_timezone() -> Option<String> {
    // Try reading /etc/localtime symlink (e.g. -> /usr/share/zoneinfo/Asia/Shanghai)
    if let Ok(target) = std::fs::read_link("/etc/localtime") {
        let path = target.to_string_lossy();
        if let Some(pos) = path.find("/zoneinfo/") {
            return Some(path[pos + "/zoneinfo/".len()..].to_string());
        }
    }

    // Try /etc/timezone (Debian/Ubuntu)
    if let Ok(tz) = std::fs::read_to_string("/etc/timezone") {
        let tz = tz.trim().to_string();
        if !tz.is_empty() {
            return Some(tz);
        }
    }

    None
}
