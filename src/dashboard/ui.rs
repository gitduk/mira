use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::module::ModuleStatus;
use super::state::DashboardState;

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn render(frame: &mut Frame, state: &DashboardState, spinner_frame: usize) {
    let size = frame.area();

    // Main layout: Header + Agent Panel + (optional) Input
    let input_height = if state.input_mode { 3 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),            // Header
            Constraint::Min(5),              // Agent panel
            Constraint::Length(input_height), // Input
        ])
        .split(size);

    render_header(frame, chunks[0], state);
    render_agent_panel(frame, chunks[1], state, spinner_frame);

    if state.input_mode {
        render_input(frame, chunks[2], state);
    }
}

fn render_header(frame: &mut Frame, area: Rect, state: &DashboardState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Mira ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    // Left column
    let left_text = vec![
        Line::from(vec![
            Span::raw("  Agent: "),
            Span::styled(&state.agent_name, Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::raw("  Groups: "),
            Span::styled(
                state.groups.len().to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  Messages: "),
            Span::styled(
                state.message_count.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  API: "),
            Span::styled(
                &state.api_base_url,
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];
    let left_paragraph = Paragraph::new(left_text);
    frame.render_widget(left_paragraph, columns[0]);

    // Right column (right-aligned)
    let mut right_lines: Vec<Line> = Vec::new();

    // Dynamic module status lines
    let mounted: Vec<_> = state.modules.iter().filter(|m| m.mounted).collect();
    if mounted.is_empty() {
        right_lines.push(Line::from(vec![
            Span::raw("Modules: "),
            Span::styled("None", Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
        ]));
    } else {
        for module in &mounted {
            let color = match module.status {
                ModuleStatus::Connected => Color::Green,
                ModuleStatus::Connecting | ModuleStatus::Reconnecting => Color::Yellow,
                ModuleStatus::Disconnected => Color::Red,
                ModuleStatus::Idle => Color::DarkGray,
            };
            let status_display = format!(
                "{} {}",
                module.status.display_symbol(),
                module.status.display_text()
            );
            right_lines.push(Line::from(vec![
                Span::raw(format!("{}: ", module.display_name)),
                Span::styled(status_display, Style::default().fg(color)),
                Span::raw("  "),
            ]));
        }
    }

    right_lines.push(Line::from(vec![
        Span::raw("Uptime: "),
        Span::raw(state.format_uptime()),
        Span::raw("  "),
    ]));
    right_lines.push(Line::from(vec![
        Span::raw("Active Agents: "),
        Span::styled(
            format!("{}/{}", state.active_agents, state.max_agents),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
    ]));
    right_lines.push(Line::from(vec![
        Span::raw("Model: "),
        Span::styled(
            &state.claude_model,
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("  "),
    ]));

    let right_paragraph = Paragraph::new(right_lines).alignment(Alignment::Right);
    frame.render_widget(right_paragraph, columns[1]);
}

fn render_agent_panel(
    frame: &mut Frame,
    area: Rect,
    state: &DashboardState,
    spinner_frame: usize,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", state.agent_name));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let height = inner.height as usize;
    if height == 0 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    if state.thinking_buffer.is_empty() && state.spinner_text.is_none() && state.streaming_line.is_none() {
        lines.push(Line::from(Span::styled(
            "  Waiting for messages...",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        // Show last N lines from thinking buffer
        let start = state.thinking_buffer.len().saturating_sub(height);
        for line in &state.thinking_buffer[start..] {
            lines.push(Line::from(format!("  {}", line)));
        }

        // Streaming line
        if let Some(ref streaming) = state.streaming_line {
            if lines.len() < height {
                lines.push(Line::from(format!("  {}", streaming)));
            }
        }

        // Spinner
        if let Some(ref spinner_text) = state.spinner_text {
            if lines.len() < height {
                let frame_char = SPINNER_FRAMES[spinner_frame % SPINNER_FRAMES.len()];
                lines.push(Line::from(Span::styled(
                    format!("  {} {}", frame_char, spinner_text),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
    }

    // Pad remaining space
    while lines.len() < height {
        lines.push(Line::from(""));
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

fn render_input(frame: &mut Frame, area: Rect, state: &DashboardState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Input ");

    let text = format!("> {}█", &state.input_buffer);
    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, area);
}
