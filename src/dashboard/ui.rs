use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthChar;

use super::state::DashboardState;
use crate::module::ModuleStatus;

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn render(frame: &mut Frame, state: &DashboardState, spinner_frame: usize) {
    let size = frame.area();

    // Main layout: Header + Agent Panel + Input (always visible when enabled)
    // Input height grows with line count: border(2) + content lines, capped at 10.
    let input_height = if state.input_enabled {
        let line_count = state.input_buffer.split('\n').count().max(1);
        (line_count as u16 + 2).min(10)
    } else {
        0
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),            // Header
            Constraint::Min(5),               // Agent panel
            Constraint::Length(input_height), // Input
        ])
        .split(size);

    render_header(frame, chunks[0], state);
    render_agent_panel(frame, chunks[1], state, spinner_frame);

    if state.input_enabled {
        render_input(frame, chunks[2], state);
    }
}

fn render_header(frame: &mut Frame, area: Rect, state: &DashboardState) {
    let block = Block::default().borders(Borders::ALL).title(" Mira ");
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
            Span::styled(
                &state.agent_name,
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
            Span::styled(&state.api_base_url, Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::raw("  Model: "),
            Span::styled(&state.claude_model, Style::default().fg(Color::DarkGray)),
        ]),
    ];
    let left_paragraph = Paragraph::new(left_text);
    frame.render_widget(left_paragraph, columns[0]);

    // Right column (right-aligned)
    let mut right_lines: Vec<Line> = Vec::new();

    // Module status summary line
    let mounted: Vec<_> = state.modules.iter().filter(|m| m.mounted).collect();
    if mounted.is_empty() {
        right_lines.push(Line::from(vec![
            Span::styled("None", Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
        ]));
    } else {
        let mut spans = Vec::new();
        for (idx, module) in mounted.iter().enumerate() {
            if idx > 0 {
                spans.push(Span::raw(", "));
            }
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
            spans.push(Span::raw(format!("{}: ", module.display_name)));
            spans.push(Span::styled(status_display, Style::default().fg(color)));
        }
        spans.push(Span::raw("  "));
        right_lines.push(Line::from(spans));
    }

    right_lines.push(Line::from(vec![
        Span::raw("Uptime: "),
        Span::raw(state.format_uptime()),
        Span::raw("  "),
    ]));
    right_lines.push(Line::from(vec![
        Span::raw("Active Tasks: "),
        Span::styled(
            state.active_tasks.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
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

    let right_paragraph = Paragraph::new(right_lines).alignment(Alignment::Right);
    frame.render_widget(right_paragraph, columns[1]);
}

fn render_agent_panel(frame: &mut Frame, area: Rect, state: &DashboardState, spinner_frame: usize) {
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

    if state.thinking_buffer.is_empty()
        && state.spinner_text.is_none()
        && state.streaming_line.is_none()
    {
        lines.push(Line::from(Span::styled(
            "  Waiting for messages...",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        let content_width = inner.width.saturating_sub(2) as usize;
        let mut wrapped: Vec<Line<'static>> = Vec::new();

        for line in &state.thinking_buffer {
            for segment in line.split('\n') {
                wrapped.extend(wrap_markdown_line(segment, content_width));
            }
        }

        if let Some(ref streaming) = state.streaming_line {
            for segment in streaming.split('\n') {
                wrapped.extend(wrap_markdown_line(segment, content_width));
            }
        }

        if let Some(ref spinner_text) = state.spinner_text {
            let frame_char = SPINNER_FRAMES[spinner_frame % SPINNER_FRAMES.len()];
            let spinner_line = format!("{} {}", frame_char, spinner_text);
            wrapped.extend(wrap_markdown_line(&spinner_line, content_width));
        }

        let max_scroll = wrapped.len().saturating_sub(height);
        let scroll_offset = state.scroll_offset.min(max_scroll);
        let end = wrapped.len().saturating_sub(scroll_offset);
        let start = end.saturating_sub(height);

        for line in &wrapped[start..end] {
            let mut spans = Vec::new();
            spans.push(Span::raw("  ".to_string()));
            spans.extend(line.spans.clone());
            lines.push(Line::from(spans));
        }
    }

    // Pad remaining space
    while lines.len() < height {
        lines.push(Line::from(""));
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

fn wrap_markdown_line(line: &str, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![Line::from("")];
    }
    if line.is_empty() {
        return vec![Line::from("")];
    }

    let spans = parse_markdown_spans(line);
    wrap_spans(spans, width)
}

fn parse_markdown_spans(line: &str) -> Vec<(String, Style)> {
    let mut spans: Vec<(String, Style)> = Vec::new();
    let mut i = 0usize;
    let bytes = line.as_bytes();

    let mut bold = false;
    let mut code = false;
    let mut buf = String::new();

    let flush = |spans: &mut Vec<(String, Style)>, text: &mut String, bold: bool, code: bool| {
        if text.is_empty() {
            return;
        }
        let mut style = Style::default();
        if bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        if code {
            style = style.fg(Color::Yellow);
        }
        spans.push((text.clone(), style));
        text.clear();
    };

    // Heading: treat lines starting with "# " as bold cyan
    if line.starts_with("# ") || line.starts_with("## ") || line.starts_with("### ") {
        return vec![(
            line.trim_start_matches('#').trim_start().to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )];
    }

    // List bullet styling
    if line.starts_with("- ") || line.starts_with("* ") {
        spans.push((line[..2].to_string(), Style::default().fg(Color::DarkGray)));
        i = 2;
    }

    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'*' {
            flush(&mut spans, &mut buf, bold, code);
            bold = !bold;
            i += 2;
            continue;
        }
        if bytes[i] == b'`' {
            flush(&mut spans, &mut buf, bold, code);
            code = !code;
            i += 1;
            continue;
        }
        let ch = line[i..].chars().next().unwrap();
        buf.push(ch);
        i += ch.len_utf8();
    }
    flush(&mut spans, &mut buf, bold, code);

    spans
}

fn wrap_spans(spans: Vec<(String, Style)>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![Line::from("")];
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut current_width = 0usize;

    let push_current = |lines: &mut Vec<Line<'static>>,
                        current: &mut Vec<Span<'static>>,
                        current_width: &mut usize| {
        lines.push(Line::from(std::mem::take(current)));
        *current_width = 0;
    };

    for (text, style) in spans {
        let mut buf = String::new();

        for ch in text.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if current_width + ch_width > width {
                if !buf.is_empty() {
                    current.push(Span::styled(buf.clone(), style));
                    buf.clear();
                }
                if !current.is_empty() {
                    push_current(&mut lines, &mut current, &mut current_width);
                }
            }

            buf.push(ch);
            current_width += ch_width;

            if current_width >= width {
                if !buf.is_empty() {
                    current.push(Span::styled(buf.clone(), style));
                    buf.clear();
                }
                push_current(&mut lines, &mut current, &mut current_width);
            }
        }

        if !buf.is_empty() {
            current.push(Span::styled(buf, style));
        }
    }

    if !current.is_empty() {
        lines.push(Line::from(current));
    }

    if lines.is_empty() {
        lines.push(Line::from(""));
    }

    lines
}

fn render_input(frame: &mut Frame, area: Rect, state: &DashboardState) {
    let block = Block::default().borders(Borders::ALL).title(" Input ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split buffer into lines, figure out which line the cursor is on
    let chars: Vec<char> = state.input_buffer.chars().collect();
    let before_cursor: String = chars[..state.cursor_pos].iter().collect();
    let cursor_line_idx = before_cursor.matches('\n').count();

    let raw_lines: Vec<&str> = state.input_buffer.split('\n').collect();
    let mut lines: Vec<Line<'_>> = Vec::new();

    // Track char offset consumed so far to locate cursor within its line
    let mut char_offset = 0;
    for (i, raw) in raw_lines.iter().enumerate() {
        let prefix = if i == 0 { "> " } else { "  " };
        let line_char_count = raw.chars().count();

        if i == cursor_line_idx {
            let cursor_in_line = state.cursor_pos - char_offset;
            let line_chars: Vec<char> = raw.chars().collect();
            let before: String = line_chars[..cursor_in_line].iter().collect();
            let after: String = line_chars[cursor_in_line..].iter().collect();
            lines.push(Line::from(vec![
                Span::styled(prefix, Style::default().fg(Color::DarkGray)),
                Span::raw(before),
                Span::styled("█", Style::default().fg(Color::Gray)),
                Span::raw(after),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(prefix, Style::default().fg(Color::DarkGray)),
                Span::raw(raw.to_string()),
            ]));
        }

        // +1 for the '\n' separator (except last line)
        char_offset += line_char_count;
        if i < raw_lines.len() - 1 {
            char_offset += 1;
        }
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}
