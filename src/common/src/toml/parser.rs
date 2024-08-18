use core::{error::Error, fmt::Display, hash::Hash};

use alloc::{string::String, vec::Vec};
use uefi::table::runtime::VariableAttributes;

use crate::hash_map::HashMap;

use super::token::TomlToken;

type Array = Vec<TomlValue>;
type Table = HashMap<String, TomlValue>;

#[derive(Debug, PartialEq)]
pub enum TomlValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Array(Array),
    Table(Table),
}

#[derive(Debug)]
pub enum TomlParserError {
    UnexpectedToken(Option<TomlToken>),
}

impl Display for TomlParserError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnexpectedToken(token) => write!(f, "Unexpected Token: {:?}", token),
        }
    }
}

impl Error for TomlParserError {}

pub struct TomlParser {
    tokens: Vec<TomlToken>,
    index: usize,
}

impl TomlValue {
    fn insert(&mut self, key: String, value: TomlValue) {
        match self {
            TomlValue::Table(ref mut table) => table.insert(key, value),
            _ => {}
        }
    }

    fn as_table(self) -> Option<Table> {
        match self {
            TomlValue::Table(table) => return Some(table),
            _ => return None,
        }
    }

    fn as_array(self) -> Option<Array> {
        match self {
            TomlValue::Array(array) => return Some(array),
            _ => return None,
        }
    }
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

    fn parse(&mut self) -> Result<TomlValue, TomlParserError> {
        let mut main_map = TomlValue::Table(HashMap::new());

        while let Some(token) = self.peek(0).cloned() {
            match token {
                TomlToken::Identifier(identifier) | TomlToken::String(identifier) => {
                    self.consume();
                    loop {
                        match self.peek(0).ok_or(TomlParserError::UnexpectedToken(None))? {
                            TomlToken::Dot => {
                                self.consume();
                                match self.peek(0).ok_or(TomlParserError::UnexpectedToken(None))? {
                                    TomlToken::Identifier(identifier)
                                    | TomlToken::String(identifier) => {
                                        self.consume();
                                        todo!();
                                    }
                                    unexpected => {
                                        return Err(TomlParserError::UnexpectedToken(Some(
                                            unexpected.clone(),
                                        )))
                                    }
                                };
                            }
                            TomlToken::Equal => {
                                break;
                            }
                            unexpected => {
                                return Err(TomlParserError::UnexpectedToken(Some(
                                    unexpected.clone(),
                                )))
                            }
                        }
                    }
                    self.expect_token(TomlToken::Equal)?;
                    let value = self.parse_value()?;
                    main_map.insert(identifier, value);
                }
                TomlToken::LBracket => {
                    self.consume();
                    let table_name = self.parse_table_name()?;
                    todo!();
                }
                unexpected => {
                    return Err(TomlParserError::UnexpectedToken(Some(unexpected.clone())))
                }
            }
        }

        return Ok(main_map);
    }

    fn parse_table_name(&mut self) -> Result<String, TomlParserError> {
        match self
            .consume()
            .ok_or(TomlParserError::UnexpectedToken(None))?
        {
            TomlToken::String(identifier) | TomlToken::Identifier(identifier) => {
                return Ok(identifier.clone());
            }
            unexpected => return Err(TomlParserError::UnexpectedToken(Some(unexpected.clone()))),
        };
    }

    fn parse_value(&mut self) -> Result<TomlValue, TomlParserError> {
        match self
            .consume()
            .ok_or(TomlParserError::UnexpectedToken(None))?
        {
            TomlToken::String(string) => return Ok(TomlValue::String(string.clone())),
            TomlToken::Interger(interger) => return Ok(TomlValue::Integer(*interger)),
            TomlToken::Boolean(boolean) => return Ok(TomlValue::Boolean(*boolean)),
            TomlToken::LBracket => return Ok(TomlValue::Array(self.parse_array()?)),
            TomlToken::LCurly => return Ok(TomlValue::Table(self.parse_inline_table()?)),
            _ => return Err(TomlParserError::UnexpectedToken(None)),
        };
    }

    fn parse_inline_table(&mut self) -> Result<Table, TomlParserError> {
        todo!();
    }

    fn parse_array(&mut self) -> Result<Array, TomlParserError> {
        todo!();
    }

    fn expect_token(&mut self, expected: TomlToken) -> Result<(), TomlParserError> {
        let token = self.peek(0).ok_or(TomlParserError::UnexpectedToken(None))?;
        if *token == expected {
            self.consume();
            return Ok(());
        } else {
            return Err(TomlParserError::UnexpectedToken(Some(token.clone())));
        }
    }
}
