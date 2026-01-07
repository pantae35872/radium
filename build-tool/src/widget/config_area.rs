use std::{fmt::Display, time::Duration};

use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::{Constraint, Layout, Margin, Size},
    style::{Color, Style, Stylize},
    symbols::scrollbar,
    text::{Line, Span, Text, ToLine},
    widgets::{
        Block, BorderType, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
        StatefulWidget, Widget, Wrap,
    },
};

use crate::widget::{CenteredParagraph, RainbowInterpolateState, clear, interpolate_rainbow, measure_text};

#[derive(Default, Debug, Clone, Copy)]
pub struct ConfigArea {
    pub delta_time: Duration,
}

impl StatefulWidget for ConfigArea {
    type State = ConfigAreaState;

    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer, state: &mut Self::State) {
        clear(area, buf);

        let Some(group_unmodified) = state.current.get_group(&state.config) else {
            return;
        };

        let Some(group) = state.current.get_group(&state.config_staging) else {
            return;
        };

        Block::bordered()
            .title(
                Line::from("config")
                    .fg(interpolate_rainbow(&mut state.rainbow_interpolate, self.delta_time))
                    .bold()
                    .centered(),
            )
            .border_style(Style::default().light_blue())
            .border_type(BorderType::Rounded)
            .render(area, buf);

        let text = Text::from(vec![
            Line::from("(ESC or q) quit | (↑↓) move up and down | (←) move up a group | (ENTER) edit or enter a group")
                .light_green(),
        ]);

        let (_width, height) = measure_text(&text, area.width - 2);

        let [help_area, config_area] =
            Layout::vertical([Constraint::Length(height + 1), Constraint::Fill(1)]).margin(1).areas(area);

        Paragraph::new(text)
            .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().light_blue()))
            .wrap(Wrap { trim: true, ..Default::default() })
            .render(help_area, buf);

        let [location_area, config_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(config_area);

        let [location_area] =
            Layout::default().constraints([Constraint::Fill(1)]).horizontal_margin(1).areas(location_area);

        Line::from(format!("At Root > {}", state.current.get_path(&state.config_staging).join(" > ")))
            .left_aligned()
            .light_cyan()
            .render(location_area, buf);

        let mut config_names = Vec::new();
        let mut config_values = Vec::new();

        for (i, (group, group_unmodified)) in group.iter().zip(group_unmodified).enumerate() {
            let name = match group {
                ConfigTree::Group { name, .. } | ConfigTree::Value { name, .. } => name,
            };
            let value_formatted = match group {
                ConfigTree::Group { .. } => "|─────> Group <─────|".to_string(),
                ConfigTree::Value { value, .. }
                    if matches!(group_unmodified, ConfigTree::Value { value: value_unmodified, .. }
                    if value_unmodified != value) =>
                {
                    format!("[ {} ]*", value)
                }
                ConfigTree::Value { value, .. } => {
                    format!("[ {} ]", value)
                }
            };

            let style =
                if state.current.index == i { Style::default().fg(Color::Cyan).bold() } else { Style::default() };

            config_names.push(name.to_line().style(style).left_aligned());
            config_values.push(Line::from(value_formatted).style(style).right_aligned());
        }

        let height = config_names.len() as u16;

        if (state.current.index + 1).saturating_sub(state.vertical_scroll) > config_area.height as usize {
            state.vertical_scroll += 1;
        }

        if (((state.current.index + 1) as i32) - state.vertical_scroll as i32) < 1 {
            state.vertical_scroll -= 1;
        }

        state.vertical_scroll_state = state
            .vertical_scroll_state
            .content_length((height.saturating_sub(config_area.height).max(1)).into())
            .position(state.vertical_scroll);

        Paragraph::new(config_names)
            .block(Block::default().padding(Padding::symmetric(4, 0)))
            .scroll((state.vertical_scroll as u16, 0))
            .render(config_area, buf);

        Paragraph::new(config_values)
            .block(Block::default().padding(Padding::symmetric(4, 0)))
            .scroll((state.vertical_scroll as u16, 0))
            .render(config_area, buf);

        Scrollbar::new(ScrollbarOrientation::VerticalRight).symbols(scrollbar::VERTICAL).render(
            config_area.outer(Margin { horizontal: 1, ..Default::default() }),
            buf,
            &mut state.vertical_scroll_state,
        );

        if let Some(ConfigTree::Value { value, name }) =
            state.edit.as_ref().and_then(|edit| edit.get_value_mut(&mut state.config_staging))
        {
            let text = match value {
                ConfigValue::Bool(value) => {
                    if *value {
                        vec![Line::from("True").style(Style::default().fg(Color::Cyan).bold()), Line::from("False")]
                    } else {
                        vec![Line::from("True"), Line::from("False").style(Style::default().fg(Color::Cyan).bold())]
                    }
                }
                ConfigValue::Number(value) => {
                    vec![Line::from(value.to_string())]
                }
                ConfigValue::Text(value) => {
                    vec![Line::from(value.clone())]
                }
                ConfigValue::Union { current, values } => {
                    let mut text = Vec::new();
                    for (i, value) in values.iter().enumerate() {
                        let style =
                            if *current == i { Style::default().fg(Color::Cyan).bold() } else { Style::default() };
                        text.push(value.to_line().style(style));
                    }
                    text
                }
            };

            CenteredParagraph::new(text)
                .block(
                    Block::bordered()
                        .title(name.to_line().centered())
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().light_blue())
                        .padding(Padding::symmetric(2.max(name.to_line().width() as u16 / 2), 0)),
                )
                .render(config_area, buf);
        }

        if let Some(wanna_save) = state.wanna_save {
            let selection = if wanna_save {
                Line::from(vec![
                    Span::styled("Save", Style::default().fg(Color::Cyan).bold()),
                    Span::from("      "),
                    Span::styled("Don't save", Style::default()),
                ])
            } else {
                Line::from(vec![
                    Span::styled("Save", Style::default()),
                    Span::from("      "),
                    Span::styled("Don't save", Style::default().fg(Color::Cyan).bold()),
                ])
            }
            .centered();
            let sline = "Save configuration and exit ?".to_line().centered();
            CenteredParagraph::new(vec![selection])
                .min_size(Size::new(sline.width() as u16 + 4, 3))
                .block(
                    Block::bordered()
                        .title(sline)
                        .title_bottom("(←→) Select, (ENTER) continue".to_line().centered())
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().light_blue())
                        .padding(Padding::symmetric(2, 1)),
                )
                .render(config_area, buf);
        }
    }
}

#[derive(Default, Debug, Clone)]
pub struct ConfigAreaState {
    pub config: Vec<ConfigTree>,

    pub config_staging: Vec<ConfigTree>,
    pub current: ConfigReference,
    pub edit: Option<ConfigReference>,
    pub wanna_save: Option<bool>,

    pub vertical_scroll_state: ScrollbarState,
    pub vertical_scroll: usize,
    pub rainbow_interpolate: RainbowInterpolateState,
}

impl ConfigAreaState {
    pub fn key_event(&mut self, event: KeyEvent) -> bool {
        if let Some(ConfigTree::Value { value, .. }) =
            self.edit.as_ref().and_then(|edit| edit.get_value_mut(&mut self.config_staging))
        {
            match (event.code, value) {
                (KeyCode::Enter | KeyCode::Esc, _) => {
                    self.edit = None;
                }
                (KeyCode::Char('h'), ConfigValue::Number(value)) if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    *value = 0;
                }
                (KeyCode::Backspace, ConfigValue::Number(value)) => {
                    *value = *value / 10;
                }
                (KeyCode::Char(c), ConfigValue::Number(value)) if let Some(digit) = c.to_digit(10) => {
                    *value = value.saturating_mul(10).saturating_add(digit as i32);
                }
                (KeyCode::Char('h'), ConfigValue::Text(value)) if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    value.clear();
                }
                (KeyCode::Backspace, ConfigValue::Text(value)) => {
                    value.pop();
                }
                (KeyCode::Char(c), ConfigValue::Text(value)) => {
                    value.push(c);
                }
                (KeyCode::Up | KeyCode::Down | KeyCode::Tab | KeyCode::BackTab, ConfigValue::Bool(value)) => {
                    *value = !*value;
                }
                (KeyCode::Up | KeyCode::BackTab, ConfigValue::Union { current, values, .. }) => {
                    if *current == 0 {
                        *current = values.len().saturating_sub(1);
                    } else {
                        *current = *current - 1;
                    }
                }
                (KeyCode::Down | KeyCode::Tab, ConfigValue::Union { current, values, .. }) => {
                    *current = (*current + 1) % values.len();
                }
                _ => {}
            }
            return false;
        }

        if let Some(value) = self.wanna_save.as_mut() {
            return match event.code {
                KeyCode::Enter => {
                    if *value {
                        self.config = self.config_staging.clone();
                    } else {
                        self.config_staging = self.config.clone();
                    }
                    self.wanna_save = None;
                    true
                }
                KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                    *value = !*value;
                    false
                }
                _ => false,
            };
        }

        match event.code {
            KeyCode::Up | KeyCode::BackTab => self.current.up(&self.config_staging),
            KeyCode::Down | KeyCode::Tab => self.current.down(&self.config_staging),
            KeyCode::Left => self.current.traverse_up(),
            KeyCode::Enter => {
                self.edit = self.current.traverse_down_or_edit(&mut self.config_staging);
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                self.wanna_save = Some(true);
            }
            _ => {}
        };
        return false;
    }
}

/// A reference to a config inside the tree, if the you want to reference a config at
/// Kernel (group 0)->Memory (group 3 inside group 0)->MaxMem (number, at index 1 at group 3 inside
/// group 0)
/// This struct will contains
/// group: [0, 3] // 0 is the kernel group then 3 in the kernel group
/// index: 1      // the index will be 1 at group 3 inside group 0
#[derive(Default, Debug, Clone)]
pub struct ConfigReference {
    group: Vec<usize>,
    index: usize,
}

impl ConfigReference {
    pub fn up(&mut self, tree: &[ConfigTree]) {
        if self.index == 0 {
            self.index = self.get_group(tree).and_then(|t| t.len().checked_sub(1)).unwrap_or(0);
            return;
        }

        self.index -= 1;
    }

    pub fn down(&mut self, tree: &[ConfigTree]) {
        self.index += 1;

        if self.get_value(tree).is_none() {
            self.index = 0;
        }
    }

    pub fn traverse_down_or_edit<'a>(&mut self, tree: &'a mut [ConfigTree]) -> Option<ConfigReference> {
        let Some(ConfigTree::Group { .. }) = self.get_value_mut(tree) else {
            return Some(self.clone());
        };

        self.group.push(self.index);
        self.index = 0;
        None
    }

    pub fn traverse_up(&mut self) {
        let Some(index) = self.group.pop() else {
            return;
        };
        self.index = index;
    }

    pub fn get_group<'a>(&self, tree: &'a [ConfigTree]) -> Option<&'a [ConfigTree]> {
        let mut current_tree = tree;

        for group_index in self.group.iter() {
            current_tree = match current_tree.get(*group_index)? {
                ConfigTree::Group { members, .. } => members,
                _ => return None,
            };
        }

        Some(current_tree)
    }

    pub fn get_group_mut<'a>(&self, tree: &'a mut [ConfigTree]) -> Option<&'a [ConfigTree]> {
        let mut current_tree = tree;

        for group_index in self.group.iter() {
            current_tree = match current_tree.get_mut(*group_index)? {
                ConfigTree::Group { members, .. } => members,
                _ => return None,
            };
        }

        Some(current_tree)
    }

    pub fn get_path(&self, tree: &[ConfigTree]) -> Vec<String> {
        let mut current_tree = tree;
        let mut path = Vec::new();

        for group_index in self.group.iter() {
            current_tree = match current_tree.get(*group_index) {
                Some(ConfigTree::Group { members, name, .. }) => {
                    path.push(format!("{name}"));
                    members
                }
                _ => return path,
            };
        }

        path
    }

    pub fn get_value<'a>(&self, tree: &'a [ConfigTree]) -> Option<&'a ConfigTree> {
        let mut current_tree = tree;

        for group_index in self.group.iter() {
            current_tree = match current_tree.get(*group_index)? {
                ConfigTree::Group { members, .. } => members,
                _ => return None,
            };
        }

        current_tree.get(self.index)
    }

    pub fn get_value_mut<'a>(&self, tree: &'a mut [ConfigTree]) -> Option<&'a mut ConfigTree> {
        let mut current_tree = tree;

        for group_index in self.group.iter() {
            current_tree = match current_tree.get_mut(*group_index)? {
                ConfigTree::Group { members, .. } => members,
                _ => return None,
            };
        }

        current_tree.get_mut(self.index)
    }
}

#[derive(Debug, Clone)]
pub enum ConfigTree {
    Group { name: String, members: Vec<ConfigTree> },
    Value { name: String, value: ConfigValue },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigValue {
    Number(i32),
    Bool(bool),
    Text(String),
    Union { current: usize, values: Vec<String> },
}

impl Display for ConfigValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Number(value) => write!(f, "{value}"),
            Self::Bool(value) => write!(f, "{value}"),
            Self::Text(value) => write!(f, "{value}"),
            Self::Union { current, values, .. } => write!(f, "{}", &values[*current]),
        }
    }
}

impl From<bool> for ConfigValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i32> for ConfigValue {
    fn from(value: i32) -> Self {
        Self::Number(value)
    }
}
