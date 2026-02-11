use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::state::DashboardState;

pub enum InputAction {
    None,
    Quit,
    Submit(String),
}

pub fn handle_key_event(event: KeyEvent, state: &mut DashboardState) -> InputAction {
    // Ctrl+C: quit
    if event.modifiers.contains(KeyModifiers::CONTROL) && event.code == KeyCode::Char('c') {
        return InputAction::Quit;
    }

    // When input is not enabled, only handle scroll keys
    if !state.input_enabled {
        match event.code {
            KeyCode::Up => {
                state.scroll_offset = state.scroll_offset.saturating_add(1);
            }
            KeyCode::Down => {
                state.scroll_offset = state.scroll_offset.saturating_sub(1);
            }
            KeyCode::PageUp => {
                state.scroll_offset = state.scroll_offset.saturating_add(10);
            }
            KeyCode::PageDown => {
                state.scroll_offset = state.scroll_offset.saturating_sub(10);
            }
            KeyCode::Home => {
                state.scroll_offset = usize::MAX;
            }
            KeyCode::End => {
                state.scroll_offset = 0;
            }
            _ => {}
        }
        return InputAction::None;
    }

    // --- Input enabled: handle editing keys ---

    // Ctrl shortcuts
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        match event.code {
            KeyCode::Char('a') => {
                // Move cursor to start
                state.cursor_pos = 0;
                return InputAction::None;
            }
            KeyCode::Char('e') => {
                // Move cursor to end
                state.cursor_pos = state.input_buffer.chars().count();
                return InputAction::None;
            }
            KeyCode::Char('u') => {
                // Kill from cursor to start
                let byte_idx = char_to_byte_idx(&state.input_buffer, state.cursor_pos);
                state.input_buffer.drain(..byte_idx);
                state.cursor_pos = 0;
                return InputAction::None;
            }
            KeyCode::Char('k') => {
                // Kill from cursor to end
                let byte_idx = char_to_byte_idx(&state.input_buffer, state.cursor_pos);
                state.input_buffer.truncate(byte_idx);
                return InputAction::None;
            }
            KeyCode::Char('w') => {
                // Ctrl+W: delete previous word, but never cross '\n'
                if state.cursor_pos > 0 {
                    let chars: Vec<char> = state.input_buffer.chars().collect();
                    let mut new_pos = state.cursor_pos;

                    // Skip horizontal whitespace before the word (but stop at newline).
                    while new_pos > 0 {
                        let ch = chars[new_pos - 1];
                        if ch == '\n' || !ch.is_whitespace() {
                            break;
                        }
                        new_pos -= 1;
                    }

                    // Delete the previous word until whitespace or newline.
                    while new_pos > 0 {
                        let ch = chars[new_pos - 1];
                        if ch == '\n' || ch.is_whitespace() {
                            break;
                        }
                        new_pos -= 1;
                    }

                    let start_byte = char_to_byte_idx(&state.input_buffer, new_pos);
                    let end_byte = char_to_byte_idx(&state.input_buffer, state.cursor_pos);
                    state.input_buffer.drain(start_byte..end_byte);
                    state.cursor_pos = new_pos;
                }
                return InputAction::None;
            }
            KeyCode::Char('j') => {
                // Ctrl+J: insert newline (Claude Code style)
                let byte_idx = char_to_byte_idx(&state.input_buffer, state.cursor_pos);
                state.input_buffer.insert(byte_idx, '\n');
                state.cursor_pos += 1;
                return InputAction::None;
            }
            KeyCode::Char('h') => {
                // Ctrl+H: delete previous character (Backspace behavior)
                if state.cursor_pos > 0 {
                    let byte_idx = char_to_byte_idx(&state.input_buffer, state.cursor_pos - 1);
                    let next_byte_idx = char_to_byte_idx(&state.input_buffer, state.cursor_pos);
                    state.input_buffer.drain(byte_idx..next_byte_idx);
                    state.cursor_pos -= 1;
                }
                return InputAction::None;
            }
            _ => {}
        }
    }

    // Shift+Up/Down for scrolling while in input mode
    if event.modifiers.contains(KeyModifiers::SHIFT) {
        match event.code {
            KeyCode::Up => {
                state.scroll_offset = state.scroll_offset.saturating_add(1);
                return InputAction::None;
            }
            KeyCode::Down => {
                state.scroll_offset = state.scroll_offset.saturating_sub(1);
                return InputAction::None;
            }
            _ => {}
        }
    }

    match event.code {
        KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
            let input = state.input_buffer.trim().to_string();
            if !input.is_empty() {
                // Add to history
                state.input_history.push(input.clone());
                state.history_index = None;
                state.saved_input.clear();
            }
            state.input_buffer.clear();
            state.cursor_pos = 0;
            if !input.is_empty() {
                return InputAction::Submit(input);
            }
        }
        KeyCode::Esc => {
            // Clear input
            state.input_buffer.clear();
            state.cursor_pos = 0;
            state.history_index = None;
            state.saved_input.clear();
        }
        KeyCode::Backspace => {
            if state.cursor_pos > 0 {
                let byte_idx = char_to_byte_idx(&state.input_buffer, state.cursor_pos - 1);
                let next_byte_idx = char_to_byte_idx(&state.input_buffer, state.cursor_pos);
                state.input_buffer.drain(byte_idx..next_byte_idx);
                state.cursor_pos -= 1;
            }
        }
        KeyCode::Delete => {
            let char_count = state.input_buffer.chars().count();
            if state.cursor_pos < char_count {
                let byte_idx = char_to_byte_idx(&state.input_buffer, state.cursor_pos);
                let next_byte_idx = char_to_byte_idx(&state.input_buffer, state.cursor_pos + 1);
                state.input_buffer.drain(byte_idx..next_byte_idx);
            }
        }
        KeyCode::Left => {
            if state.cursor_pos > 0 {
                state.cursor_pos -= 1;
            }
        }
        KeyCode::Right => {
            if state.cursor_pos < state.input_buffer.chars().count() {
                state.cursor_pos += 1;
            }
        }
        KeyCode::Home => {
            state.cursor_pos = 0;
        }
        KeyCode::End => {
            state.cursor_pos = state.input_buffer.chars().count();
        }
        KeyCode::Up => {
            // Navigate history (older)
            let history_len = state.input_history.len();
            if history_len > 0 {
                match state.history_index {
                    None => {
                        // Save current input and go to most recent history
                        state.saved_input = state.input_buffer.clone();
                        state.history_index = Some(history_len - 1);
                        state.input_buffer = state.input_history[history_len - 1].clone();
                        state.cursor_pos = state.input_buffer.chars().count();
                    }
                    Some(idx) if idx > 0 => {
                        state.history_index = Some(idx - 1);
                        state.input_buffer = state.input_history[idx - 1].clone();
                        state.cursor_pos = state.input_buffer.chars().count();
                    }
                    _ => {}
                }
            }
        }
        KeyCode::Down => {
            // Navigate history (newer)
            if let Some(idx) = state.history_index {
                let history_len = state.input_history.len();
                if idx + 1 < history_len {
                    state.history_index = Some(idx + 1);
                    state.input_buffer = state.input_history[idx + 1].clone();
                    state.cursor_pos = state.input_buffer.chars().count();
                } else {
                    // Restore saved input
                    state.history_index = None;
                    state.input_buffer = state.saved_input.clone();
                    state.cursor_pos = state.input_buffer.chars().count();
                    state.saved_input.clear();
                }
            }
        }
        KeyCode::Char(c) => {
            let byte_idx = char_to_byte_idx(&state.input_buffer, state.cursor_pos);
            state.input_buffer.insert(byte_idx, c);
            state.cursor_pos += 1;
        }
        _ => {}
    }

    InputAction::None
}

/// Convert a character index to a byte index in a string.
fn char_to_byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}
