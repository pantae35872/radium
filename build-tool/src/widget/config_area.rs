use std::time::Duration;

use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::{Constraint, Flex, Layout, Margin},
    style::{Color, Style, Stylize},
    symbols::scrollbar,
    text::{Line, Text, ToLine},
    widgets::{
        Block, BorderType, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
        StatefulWidget, Widget, Wrap,
    },
};

use crate::{
    config::{ConfigRoot, ConfigTree, ConfigValue},
    widget::{CenteredParagraph, RainbowInterpolateState, clear, interpolate_rainbow, measure_text},
};

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
            let (name, overwriting_name) = match group {
                ConfigTree::Group { name, overwriting_name, .. } | ConfigTree::Value { name, overwriting_name, .. } => {
                    (name, overwriting_name)
                }
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

            config_names.push(Line::from(format!("{name} ({overwriting_name})")).style(style).left_aligned());
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

        if let Some(ConfigTree::Value { value, name, .. }) =
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
            let mut content = Vec::new();
            if let Some(configs) = state.changed_diff.as_ref() {
                for config in configs.iter() {
                    let old_value = match config.get_value(&state.config) {
                        Some(ConfigTree::Value { value, .. }) => value.to_string(),
                        _ => Default::default(),
                    };
                    let new_value = match config.get_value(&state.config_staging) {
                        Some(ConfigTree::Value { value, .. }) => value.to_string(),
                        _ => Default::default(),
                    };
                    content.push(Line::from(format!(
                        "`{}`: from `{old_value}` to `{new_value}`",
                        config.format_path(&state.config),
                    )));
                }
            }

            let [bordered_area] = Layout::vertical([Constraint::Percentage(80)]).flex(Flex::Center).areas(area);
            let [bordered_area] = Layout::vertical([Constraint::Length(content.len() as u16 + 4)])
                .flex(Flex::Center)
                .areas(bordered_area);
            let [bordered_area] =
                Layout::horizontal([Constraint::Percentage(70)]).flex(Flex::Center).areas(bordered_area);

            clear(bordered_area, buf);

            Block::bordered()
                .title("Save configuration and exit ?".to_line().centered())
                .title_bottom("(←→) Select, (ENTER) continue, (ESC) cancel".to_line().centered())
                .border_type(BorderType::Rounded)
                .border_style(Style::default().light_blue())
                .render(bordered_area, buf);

            let [area] = Layout::default().constraints([Constraint::Fill(1)]).margin(1).areas(bordered_area);
            let [header_area, diff_area, selection_area] =
                Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1)]).areas(area);
            "Changes".to_line().centered().render(header_area, buf);
            let (save, dontsave) = (Line::from("Save"), Line::from("Don't save"));

            let [_, save_area, _, dontsave_area, _] = Layout::horizontal([
                Constraint::Percentage(20),
                Constraint::Length(save.width() as u16),
                Constraint::Fill(1),
                Constraint::Length(dontsave.width() as u16),
                Constraint::Percentage(20),
            ])
            .areas(selection_area);

            if wanna_save {
                save.style(Style::default().fg(Color::Cyan).bold()).render(save_area, buf);
                dontsave.render(dontsave_area, buf);
            } else {
                save.render(save_area, buf);
                dontsave.style(Style::default().fg(Color::Cyan).bold()).render(dontsave_area, buf);
            }

            let height = content.len();
            let max_scroll = height.saturating_sub(diff_area.height as usize);
            if state.diff_scroll > max_scroll {
                state.diff_scroll = max_scroll;
            }
            state.diff_scroll_state = state.diff_scroll_state.content_length(max_scroll).position(state.diff_scroll);

            Scrollbar::new(ScrollbarOrientation::VerticalRight).symbols(scrollbar::VERTICAL).render(
                diff_area.outer(Margin { horizontal: 1, ..Default::default() }),
                buf,
                &mut state.diff_scroll_state,
            );

            let [diff_area] = Layout::horizontal([Constraint::Percentage(95)]).flex(Flex::Center).areas(diff_area);
            Paragraph::new(content).scroll((state.diff_scroll as u16, 0)).render(diff_area, buf);
        }
    }
}

#[derive(Default, Debug, Clone)]
pub struct ConfigAreaState {
    pub config: Vec<ConfigTree>,
    pub config_staging: Vec<ConfigTree>,

    // Main menu
    pub vertical_scroll_state: ScrollbarState,
    pub vertical_scroll: usize,
    pub current: ConfigReference,

    // Editing menu
    pub edit: Option<ConfigReference>,

    // Save exit menu
    pub changed_diff: Option<Vec<ConfigReference>>,
    pub wanna_save: Option<bool>,
    pub diff_scroll_state: ScrollbarState,
    pub diff_scroll: usize,

    // Misc
    pub rainbow_interpolate: RainbowInterpolateState,
}

impl ConfigAreaState {
    pub fn take_changed_diff(&mut self) {
        fn append_diff(diff: &mut Vec<ConfigReference>, pos: Vec<usize>, tree_1: &[ConfigTree], tree_2: &[ConfigTree]) {
            for (i, (tree_1, tree_2)) in tree_1.iter().zip(tree_2).enumerate() {
                match (tree_1, tree_2) {
                    (ConfigTree::Group { members: members_1, .. }, ConfigTree::Group { members: members_2, .. }) => {
                        let mut new_pos = pos.clone();
                        new_pos.push(i);
                        append_diff(diff, new_pos, members_1, members_2);
                    }
                    (ConfigTree::Value { value: value_1, .. }, ConfigTree::Value { value: value_2, .. })
                        if value_1 != value_2 =>
                    {
                        diff.push(ConfigReference { group: pos.clone(), index: i });
                    }
                    _ => {}
                }
            }
        }

        let mut diff = Vec::new();
        append_diff(&mut diff, Vec::new(), &self.config, &self.config_staging);
        self.changed_diff = Some(diff);
    }

    pub fn key_event(&mut self, event: KeyEvent) -> Option<ConfigRoot> {
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
            return None;
        }

        if let Some(value) = self.wanna_save.as_mut() {
            return match event.code {
                KeyCode::Esc => {
                    self.wanna_save = None;
                    None
                }
                KeyCode::Up => {
                    self.diff_scroll = self.diff_scroll.saturating_sub(1);
                    None
                }
                KeyCode::Down => {
                    self.diff_scroll += 1;
                    None
                }
                KeyCode::Enter => {
                    if *value {
                        self.config = self.config_staging.clone();
                    } else {
                        self.config_staging = self.config.clone();
                    }
                    self.wanna_save = None;
                    self.config.clone().try_into().ok()
                }
                KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                    *value = !*value;
                    None
                }
                _ => None,
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
                self.take_changed_diff();
                if self.changed_diff.as_ref().is_some_and(|d| d.is_empty()) {
                    return self.config.clone().try_into().ok();
                }
                self.wanna_save = Some(true);
            }
            _ => {}
        };

        None
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
    pub fn format_path(&self, tree: &[ConfigTree]) -> String {
        let mut current_tree = tree;
        let mut names = vec!["Root".to_string()];

        for group_index in self.group.iter() {
            current_tree = match current_tree.get(*group_index) {
                Some(ConfigTree::Group { members, name, .. }) => {
                    names.push(name.clone());
                    members
                }
                _ => return names.join("->"),
            };
        }

        names.push(
            current_tree
                .get(self.index)
                .map(|e| match e {
                    ConfigTree::Group { name, .. } | ConfigTree::Value { name, .. } => name.clone(),
                })
                .unwrap_or_default(),
        );
        names.join("->")
    }

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

    pub fn get_path(&self, tree: &[ConfigTree]) -> Vec<String> {
        let mut current_tree = tree;
        let mut path = Vec::new();

        for group_index in self.group.iter() {
            current_tree = match current_tree.get(*group_index) {
                Some(ConfigTree::Group { members, name, overwriting_name, .. }) => {
                    path.push(format!("{name} ({overwriting_name})"));
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
