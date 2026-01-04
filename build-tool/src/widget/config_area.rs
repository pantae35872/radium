use ratatui::{
    crossterm::event::{KeyCode, KeyEvent},
    style::{Color, Stylize},
    text::Line,
    widgets::{Block, BorderType, Padding, Paragraph, StatefulWidget, Widget},
};

#[derive(Default, Debug, Clone, Copy)]
pub struct ConfigArea;

impl StatefulWidget for ConfigArea {
    type State = ConfigAreaState;

    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer, state: &mut Self::State) {
        let Some(group) = state.current.get_group(&state.configs) else {
            return;
        };

        let mut configs = Vec::new();
        for (i, group) in group.iter().enumerate() {
            let name = match group {
                ConfigTree::Number { name, .. } | ConfigTree::Group { name, .. } | ConfigTree::Bool { name, .. } => {
                    name
                }
            };
            let line = Line::from(name.as_str());
            if state.current.index == i {
                configs.push(line.bg(Color::White));
            } else {
                configs.push(line);
            }
        }

        Paragraph::new(configs)
            .block(
                Block::bordered()
                    .title(Line::from("config").centered())
                    .title_bottom(
                        Line::from("(ESC or q) quit | (↑) move up | (↓) move down | (ENTER) to edit").centered().bold(),
                    )
                    .light_blue()
                    .border_type(BorderType::Rounded)
                    .padding(Padding::symmetric(1, 1)),
            )
            .render(area, buf);
    }
}

#[derive(Default, Debug, Clone)]
pub struct ConfigAreaState {
    pub configs: Vec<ConfigTree>,
    pub current: ConfigReference,
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
