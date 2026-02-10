use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::state::DashboardState;

pub enum InputAction {
    None,
    Quit,
    ToggleInput,
    Submit(String),
}

pub fn handle_key_event(event: KeyEvent, state: &mut DashboardState) -> InputAction {
    // Ctrl+C: quit
    if event.modifiers.contains(KeyModifiers::CONTROL) && event.code == KeyCode::Char('c') {
        return InputAction::Quit;
    }

    // Ctrl+D: toggle input mode
    if event.modifiers.contains(KeyModifiers::CONTROL) && event.code == KeyCode::Char('d') {
        state.input_mode = !state.input_mode;
        if !state.input_mode {
            state.input_buffer.clear();
            state.cursor_pos = 0;
        }
        return InputAction::ToggleInput;
    }

    if !state.input_mode {
        return InputAction::None;
    }

    match event.code {
        KeyCode::Enter => {
            let input = state.input_buffer.trim().to_string();
            state.input_buffer.clear();
            state.cursor_pos = 0;
            if !input.is_empty() {
                return InputAction::Submit(input);
            }
        }
        KeyCode::Backspace => {
            if state.cursor_pos > 0 {
                let byte_idx = char_to_byte_idx(&state.input_buffer, state.cursor_pos - 1);
                state.input_buffer.remove(byte_idx);
                state.cursor_pos -= 1;
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
