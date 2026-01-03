use std::time::Duration;

use arboard::Clipboard;
use oklab::{LinearRgb, Oklab};
use ratatui::{
    Frame,
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Position, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, BorderType, Paragraph, StatefulWidget, Widget},
};

use crate::widget::prompt::interpolate::interpolate_multiple;

mod interpolate;

#[derive(Default, Debug, Clone, Copy)]
pub struct Promt<'s> {
    pub running_cmd: &'s str,
    pub command_status: CommandStatus,
    pub delta_time: Duration,
}

#[derive(Debug, Default, Clone, Copy)]
pub enum CommandStatus {
    /// Display a green promt if idle
    #[default]
    Idle,
    /// Interpolate rainbow colors, if the command is running
    Busy,
    /// Display a red promt if the command, failed to execute
    Errored,
}

#[derive(Default, Debug)]
pub struct PromtState {
    history: Vec<String>,
    current_input: usize,
    character_index: usize,
    rainbow_interpolate: usize,
}

impl PromtState {
    fn clamp_cursor(&self, new_cursor_pos: usize) -> usize {
        new_cursor_pos.clamp(0, self.input().chars().count())
    }

    fn move_cursor_right(&mut self) {
        let cursor_moved_right = self.character_index.saturating_add(1);
        self.character_index = self.clamp_cursor(cursor_moved_right);
    }

    fn move_cursor_left(&mut self) {
        let cursor_moved_left = self.character_index.saturating_sub(1);
        self.character_index = self.clamp_cursor(cursor_moved_left);
    }

    fn input(&self) -> &str {
        &self.history[self.current_input]
    }

    fn expand_history(&mut self) {
        if self.current_input >= self.history.len() {
            self.history.push(String::new());
        }
    }

    fn input_mut(&mut self) -> &mut String {
        self.expand_history();

        &mut self.history[self.current_input]
    }

    fn byte_index(&self) -> usize {
        self.input().char_indices().map(|(i, _)| i).nth(self.character_index).unwrap_or(self.input().len())
    }

    fn len_index(&self) -> usize {
        self.input().char_indices().map(|(i, _)| i).last().unwrap_or(self.input().len())
    }

    fn delete_char_back(&mut self) {
        let is_cursor_rightmost = self.character_index > self.len_index();
        if !is_cursor_rightmost {
            let current_index = self.character_index;
            let from_left_to_current_index = current_index;
            let before_char_to_delete = self.input().chars().take(from_left_to_current_index);
            let after_char_to_delete = self.input().chars().skip(current_index + 1);
            *self.input_mut() = before_char_to_delete.chain(after_char_to_delete).collect();
        }
    }

    fn delete_char(&mut self) {
        let is_not_cursor_leftmost = self.character_index != 0;
        if is_not_cursor_leftmost {
            let current_index = self.character_index;
            let from_left_to_current_index = current_index - 1;
            let before_char_to_delete = self.input().chars().take(from_left_to_current_index);
            let after_char_to_delete = self.input().chars().skip(current_index);
            *self.input_mut() = before_char_to_delete.chain(after_char_to_delete).collect();
            self.move_cursor_left();
        }
    }

    fn paste_string(&mut self, new_str: &str) {
        let index = self.byte_index();
        self.input_mut().insert_str(index, new_str);
        self.character_index = self.clamp_cursor(self.len_index() + 1);
    }

    fn enter_char(&mut self, new_char: char) {
        let index = self.byte_index();
        self.input_mut().insert(index, new_char);
        self.move_cursor_right();
    }

    pub fn set_cursor_pos(&self, area: Rect, frame: &mut Frame) {
        frame.set_cursor_position(Position::new(area.x + self.character_index as u16 + 4, area.y + 1));
    }

    pub fn key_event(&mut self, event: KeyEvent) -> Option<String> {
        self.expand_history();

        match event.kind {
            KeyEventKind::Press => match event.code {
                KeyCode::Char('v')
                    if event.modifiers.contains(KeyModifiers::CONTROL)
                        && let Ok(text) = Clipboard::new().and_then(|mut c| c.get_text()) =>
                {
                    self.paste_string(text.as_str());
                }
                KeyCode::Char('r') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    todo!("Reverse search!")
                }
                KeyCode::Enter => {
                    let ret = self.input_mut().clone();
                    if self.current_input == self.history.len() - 1 {
                        self.history.push(String::new());
                        self.current_input = self.history.len() - 1;
                    } else {
                        self.history.insert(self.history.len() - 1, self.input().to_string());
                        self.current_input = self.history.len() - 1;
                        self.input_mut().clear();
                    }

                    self.character_index = 0;
                    return Some(ret);
                }
                KeyCode::Char(to_insert) => self.enter_char(to_insert),
                KeyCode::Backspace => self.delete_char(),
                KeyCode::Delete => self.delete_char_back(),
                KeyCode::Left => self.move_cursor_left(),
                KeyCode::Right => self.move_cursor_right(),
                KeyCode::Up => {
                    self.current_input = self.current_input.saturating_sub(1);
                    self.character_index = self.input().len();
                }
                KeyCode::Down => {
                    self.current_input = self.current_input.saturating_add(1).clamp(0, self.history.len() - 1);
                    self.character_index = self.input().len();
                }
                _ => {}
            },
            _ => {}
        }
        None
    }
}

impl StatefulWidget for Promt<'_> {
    type State = PromtState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let rainbow_interpolate = interpolate_multiple(
            [
                Oklab::from_linear_rgb(LinearRgb::new(1.0, 0.0, 0.0)), // Red
                Oklab::from_linear_rgb(LinearRgb::new(1.0, 1.0, 0.0)), // Yellow
                Oklab::from_linear_rgb(LinearRgb::new(0.0, 1.0, 0.0)), // Green
                Oklab::from_linear_rgb(LinearRgb::new(0.0, 1.0, 1.0)), // Cyan
                Oklab::from_linear_rgb(LinearRgb::new(0.0, 0.0, 1.0)), // Blue
                Oklab::from_linear_rgb(LinearRgb::new(1.0, 0.0, 1.0)), // Magenta
            ],
            state.rainbow_interpolate as f32 / 10_000.0,
        )
        .to_srgb();
        let color = match self.command_status {
            CommandStatus::Idle => Color::Rgb(44, 255, 5),
            CommandStatus::Busy => Color::Rgb(rainbow_interpolate.r, rainbow_interpolate.g, rainbow_interpolate.b),
            CommandStatus::Errored => Color::LightRed,
        };

        state.expand_history();

        Paragraph::new(format!(" > {}", state.input()))
            .block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(color))
                    .title(Line::from("Promt").left_aligned())
                    .title(Line::from(self.running_cmd).centered()),
            )
            .render(area, buf);

        state.rainbow_interpolate = (state.rainbow_interpolate + 1 + self.delta_time.as_millis() as usize) % 10000;
    }
}
