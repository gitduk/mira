use crate::module::Module;

pub struct DashboardState {
    pub agent_name: String,
    pub max_agents: usize,
    pub modules: Vec<Module>,
    pub active_agents: usize,
    pub active_tasks: usize,
    pub message_count: usize,
    pub start_time: std::time::Instant,
    pub thinking_buffer: Vec<String>,
    pub spinner_text: Option<String>,
    pub streaming_line: Option<String>,
    pub input_buffer: String,
    pub cursor_pos: usize,
    pub input_enabled: bool,
    pub api_base_url: String,
    pub claude_model: String,
    pub max_thinking_lines: usize,
    pub scroll_offset: usize,
    pub input_history: Vec<String>,
    pub history_index: Option<usize>,
    pub saved_input: String,
}

impl DashboardState {
    pub fn new(
        agent_name: String,
        max_agents: usize,
        api_base_url: String,
        claude_model: String,
        max_thinking_lines: usize,
    ) -> Self {
        DashboardState {
            agent_name,
            max_agents,
            modules: Vec::new(),
            active_agents: 0,
            active_tasks: 0,
            message_count: 0,
            start_time: std::time::Instant::now(),
            thinking_buffer: Vec::new(),
            spinner_text: None,
            streaming_line: None,
            input_buffer: String::new(),
            cursor_pos: 0,
            input_enabled: false,
            api_base_url,
            claude_model,
            max_thinking_lines,
            scroll_offset: 0,
            input_history: Vec::new(),
            history_index: None,
            saved_input: String::new(),
        }
    }

    pub fn push_thinking_line(&mut self, line: String) {
        self.thinking_buffer.push(line);
        // Auto-follow new output
        self.scroll_offset = 0;
        if self.thinking_buffer.len() > self.max_thinking_lines {
            let overflow = self.thinking_buffer.len() - self.max_thinking_lines;
            self.thinking_buffer.drain(0..overflow);
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
