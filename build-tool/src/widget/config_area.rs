use std::time::Duration;

use ratatui::{
    crossterm::event::{KeyCode, KeyEvent},
    layout::{Constraint, Layout, Margin},
    style::{Color, Style, Stylize},
    symbols::scrollbar,
    text::{Line, Text},
    widgets::{
        Block, BorderType, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
        StatefulWidget, Widget, Wrap,
    },
};

use crate::widget::{RainbowInterpolateState, interpolate_rainbow, measure_text};

#[derive(Default, Debug, Clone, Copy)]
pub struct ConfigArea {
    pub delta_time: Duration,
}

impl StatefulWidget for ConfigArea {
    type State = ConfigAreaState;

    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer, state: &mut Self::State) {
        let Some(group) = state.current.get_group(&state.configs) else {
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

        Line::from(format!("At Root > {}", state.current.get_path(&state.configs).join(" > ")))
            .left_aligned()
            .light_cyan()
            .render(location_area, buf);

        let mut configs = Vec::new();

        for (i, group) in group.iter().enumerate() {
            let name = match group {
                ConfigTree::Number { name, .. } | ConfigTree::Group { name, .. } | ConfigTree::Bool { name, .. } => {
                    name
                }
            };
            let line = Line::from(name.as_str());
            if state.current.index == i {
                configs.push(line.style(Style::default().fg(Color::Cyan).bold()));
            } else {
                configs.push(line);
            }
        }

        let text = Text::from(configs);
        let (_width, height) = measure_text(&text, config_area.width);

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

        Paragraph::new(text)
            .block(Block::default().padding(Padding::symmetric(4, 0)))
            .scroll((state.vertical_scroll as u16, 0))
            .render(config_area, buf);
        Scrollbar::new(ScrollbarOrientation::VerticalRight).symbols(scrollbar::VERTICAL).render(
            config_area.outer(Margin { horizontal: 1, ..Default::default() }),
            buf,
            &mut state.vertical_scroll_state,
        );
    }
}

#[derive(Default, Debug, Clone)]
pub struct ConfigAreaState {
    pub configs: Vec<ConfigTree>,
    pub current: ConfigReference,
    pub vertical_scroll_state: ScrollbarState,
    pub vertical_scroll: usize,
    pub rainbow_interpolate: RainbowInterpolateState,
}

impl ConfigAreaState {
    pub fn key_event(&mut self, event: KeyEvent) {
        match event.code {
            KeyCode::Up => self.current.up(&self.configs),
            KeyCode::Down => self.current.down(&self.configs),
            KeyCode::Left => self.current.traverse_up(),
            KeyCode::Enter => self.current.traverse_down(&self.configs),
            _ => {}
        };
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
            return;
        }

        self.index -= 1;
    }

    pub fn down(&mut self, tree: &[ConfigTree]) {
        self.index += 1;

        if self.get_value(tree).is_none() {
            self.index -= 1;
        }
    }

    pub fn traverse_down(&mut self, tree: &[ConfigTree]) {
        let Some(ConfigTree::Group { .. }) = self.get_value(tree) else {
            return;
        };
        self.group.push(self.index);
        self.index = 0;
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

    pub fn get_mut_value<'a>(&self, tree: &'a mut [ConfigTree]) -> Option<&'a mut ConfigTree> {
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
    Number { name: String, value: i32 },
    Bool { name: String, value: bool },
}
