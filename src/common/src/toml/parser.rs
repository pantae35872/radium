use alloc::{string::String, vec::Vec};

use crate::hash_map::HashMap;

use super::token::TomlToken;

#[derive(Debug, PartialEq)]
enum TomlValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Array(Vec<TomlValue>),
    Table(HashMap<String, TomlValue>),
}

pub enum TomlParserError {}

pub struct TomlParser {
    tokens: Vec<TomlToken>,
    index: usize,
}

impl TomlParser {
    fn peek(&self, offset: usize) -> Option<&TomlToken> {
        return self.tokens.get(self.index + offset);
    }

    fn consume(&mut self) -> Option<&TomlToken> {
        if let Some(token) = self.tokens.get(self.index) {
            self.index += 1;
            return Some(token);
        } else {
            return None;
        }
    }

    fn parse(&mut self) -> Result<HashMap<String, TomlValue>, TomlParserError> {
        todo!()
    }
}
