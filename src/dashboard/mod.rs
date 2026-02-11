pub mod input;
pub mod state;
pub mod ui;

use std::io;
use std::time::Duration;

use crossterm::event::{
    self, Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::{mpsc, watch};

use crate::module::{Module, ModuleStatus};
use input::InputAction;
use state::DashboardState;

const RENDER_INTERVAL_MS: u64 = 50; // ~20fps

pub enum DashboardCommand {
    SetModules(Vec<Module>),
    SetModuleStatus(String, ModuleStatus),
    SetModuleMounted(String, bool),
    SetActiveAgents(usize),
    SetActiveTasks(usize),
    IncrementMessages(usize),
    PushThinkingLine(String),
    SetThinkingIndicator(Option<String>),
    SetStreamingLine(Option<String>),
    EnableInputMode,
}

pub struct Dashboard {
    state: DashboardState,
    cmd_rx: mpsc::Receiver<DashboardCommand>,
    shutdown_rx: watch::Receiver<bool>,
    input_tx: Option<mpsc::Sender<String>>,
    spinner_frame: usize,
}

impl Dashboard {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agent_name: String,
        max_agents: usize,
        api_base_url: String,
        claude_model: String,
        max_thinking_lines: usize,
        cmd_rx: mpsc::Receiver<DashboardCommand>,
        shutdown_rx: watch::Receiver<bool>,
        input_tx: Option<mpsc::Sender<String>>,
    ) -> Self {
        Dashboard {
            state: DashboardState::new(
                agent_name,
                max_agents,
                api_base_url,
                claude_model,
                max_thinking_lines,
            ),
            cmd_rx,
            shutdown_rx,
            input_tx,
            spinner_frame: 0,
        }
    }

    pub fn run(mut self) -> io::Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        io::stdout().execute(EnterAlternateScreen)?;
        // Try to enable key disambiguation so Shift+Enter can be distinguished from Enter.
        let _ = io::stdout().execute(PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES,
        ));
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        let tick_rate = Duration::from_millis(RENDER_INTERVAL_MS);

        loop {
            // Check shutdown
            if *self.shutdown_rx.borrow() {
                break;
            }

            // Process all pending commands
            while let Ok(cmd) = self.cmd_rx.try_recv() {
                self.process_command(cmd);
            }

            // Render
            terminal.draw(|frame| {
                ui::render(frame, &self.state, self.spinner_frame);
            })?;

            // Handle input events
            if event::poll(tick_rate)? {
                if let Event::Key(key_event) = event::read()? {
                    if key_event.kind != KeyEventKind::Press
                        && key_event.kind != KeyEventKind::Repeat
                    {
                        continue;
                    }
                    match input::handle_key_event(key_event, &mut self.state) {
                        InputAction::Quit => break,
                        InputAction::Submit(text) => {
                            if let Some(ref tx) = self.input_tx {
                                let _ = tx.try_send(text);
                            }
                        }
                        InputAction::None => {}
                    }
                }
            }

            // Advance spinner
            if self.state.spinner_text.is_some() {
                self.spinner_frame = (self.spinner_frame + 1) % 10;
            }
        }

        // Restore terminal
        let _ = io::stdout().execute(PopKeyboardEnhancementFlags);
        disable_raw_mode()?;
        io::stdout().execute(LeaveAlternateScreen)?;

        Ok(())
    }

    fn process_command(&mut self, cmd: DashboardCommand) {
        match cmd {
            DashboardCommand::SetModules(modules) => {
                self.state.modules = modules;
            }
            DashboardCommand::SetModuleStatus(id, status) => {
                if let Some(m) = self.state.modules.iter_mut().find(|m| m.id == id) {
                    m.status = status;
                }
            }
            DashboardCommand::SetModuleMounted(id, mounted) => {
                if let Some(m) = self.state.modules.iter_mut().find(|m| m.id == id) {
                    m.mounted = mounted;
                }
            }
            DashboardCommand::SetActiveAgents(count) => {
                self.state.active_agents = count;
            }
            DashboardCommand::SetActiveTasks(count) => {
                self.state.active_tasks = count;
            }
            DashboardCommand::IncrementMessages(count) => {
                self.state.message_count += count;
            }
            DashboardCommand::PushThinkingLine(line) => {
                self.state.push_thinking_line(line);
            }
            DashboardCommand::SetThinkingIndicator(text) => {
                self.state.spinner_text = text;
            }
            DashboardCommand::SetStreamingLine(line) => {
                self.state.streaming_line = line;
            }
            DashboardCommand::EnableInputMode => {
                self.state.input_enabled = true;
            }
        }
    }
}
