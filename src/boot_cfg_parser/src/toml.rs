use core::{error::Error, fmt::Display};

use lexer::{TomlLexer, TomlLexerError};
use parser::{TomlParser, TomlParserError, TomlValue};

pub mod lexer;
pub mod parser;
pub mod token;

#[derive(Debug)]
pub enum TomlError {
    TomlLexerError(TomlLexerError),
    TomlParserError(TomlParserError),
}

impl Display for TomlError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TomlLexerError(lexer_error) => {
                write!(f, "Trying to parse with lexer error: {}", lexer_error)
            }
            Self::TomlParserError(parser_error) => {
                write!(f, "Trying to parse with parser error: {}", parser_error)
            }
        }
    }
}

impl Error for TomlError {}

pub fn parse_toml(value: &str) -> Result<TomlValue, TomlError> {
    let lexer = TomlLexer::new(value);
    let token = lexer.tokenize().map_err(TomlError::TomlLexerError)?;
    let mut parser = TomlParser::new(token);
    return Ok(parser.parse().map_err(TomlError::TomlParserError)?);
}
