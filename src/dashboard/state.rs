use std::collections::HashMap;

use crate::module::Module;

#[derive(Debug, Clone, PartialEq)]
pub enum GroupDisplayStatus {
    Idle,
    Running,
    Queued,
}

#[derive(Debug, Clone)]
pub struct GroupDisplay {
    pub name: String,
    pub folder: String,
    pub status: GroupDisplayStatus,
}

pub struct DashboardState {
    pub agent_name: String,
    pub max_agents: usize,
    pub modules: Vec<Module>,
    pub groups: HashMap<String, GroupDisplay>,
    pub active_agents: usize,
    pub message_count: usize,
    pub start_time: std::time::Instant,
    pub thinking_buffer: Vec<String>,
    pub spinner_text: Option<String>,
    pub streaming_line: Option<String>,
    pub input_buffer: String,
    pub cursor_pos: usize,
    pub input_mode: bool,
    pub api_base_url: String,
    pub claude_model: String,
}

impl DashboardState {
    pub fn new(agent_name: String, max_agents: usize, api_base_url: String, claude_model: String) -> Self {
        DashboardState {
            agent_name,
            max_agents,
            modules: Vec::new(),
            groups: HashMap::new(),
            active_agents: 0,
            message_count: 0,
            start_time: std::time::Instant::now(),
            thinking_buffer: Vec::new(),
            spinner_text: None,
            streaming_line: None,
            input_buffer: String::new(),
            cursor_pos: 0,
            input_mode: false,
            api_base_url,
            claude_model,
        }
    }

    pub fn push_thinking_line(&mut self, line: String) {
        self.thinking_buffer.push(line);
        if self.thinking_buffer.len() > 500 {
            self.thinking_buffer.drain(0..self.thinking_buffer.len() - 500);
        }
    }

    pub fn format_uptime(&self) -> String {
        let elapsed = self.start_time.elapsed();
        let seconds = elapsed.as_secs();
        let minutes = seconds / 60;
        let hours = minutes / 60;
        let days = hours / 24;

        if days > 0 {
            format!("{}d {}h", days, hours % 24)
        } else if hours > 0 {
            format!("{}h {}m", hours, minutes % 60)
        } else if minutes > 0 {
            format!("{}m {}s", minutes, seconds % 60)
        } else {
            format!("{}s", seconds)
        }
    }
}
