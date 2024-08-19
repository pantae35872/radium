use core::{error::Error, fmt::Display, num::ParseIntError};

use alloc::{format, string::String, vec::Vec};

use super::token::TomlToken;

pub struct TomlLexer<'a> {
    buffer: &'a str,
    index: usize,
}

#[derive(Debug)]
pub enum TomlLexerError {
    InvalidInterger(ParseIntError),
    InvalidToken(String),
    InvalidEscapeSequence(String),
    ExpectedEndDoubleQuote(Option<char>),
    ExpectedEndSingleQuote(Option<char>),
    FloatNumberNotSupport,
    EndOfBuffer,
}

impl Display for TomlLexerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidInterger(e) => write!(f, "Trying to tokenize invalid number, {}", e),
            Self::InvalidEscapeSequence(escape_sequence) => {
                write!(f, "Invalid escape sequence: '{}'", escape_sequence)
            }
            Self::InvalidToken(invalid_token) => {
                write!(f, "Invalid token: {}", invalid_token)
            }
            Self::ExpectedEndDoubleQuote(found) => {
                write!(f, "Expected end double quote found: {:?}", found)
            }
            Self::ExpectedEndSingleQuote(found) => {
                write!(f, "Expected end single quote, found: {:?}", found)
            }
            Self::FloatNumberNotSupport => write!(f, "float nuber are not support in this parser"),
            Self::EndOfBuffer => write!(
                f,
                "trying to read next token but already at the end of the buffer"
            ),
        }
    }
}

impl Error for TomlLexerError {}

impl<'a> TomlLexer<'a> {
    pub fn new(buffer: &'a str) -> Self {
        Self { buffer, index: 0 }
    }

    fn peek(&self, offset: usize) -> Option<char> {
        return self
            .buffer
            .as_bytes()
            .get(self.index + offset)
            .map(|&b| b as char);
    }

    fn consume(&mut self) -> Option<char> {
        if let Some(&byte) = self.buffer.as_bytes().get(self.index) {
            self.index += 1;
            return Some(byte as char);
        } else {
            return None;
        }
    }

    pub fn tokenize(mut self) -> Result<Vec<TomlToken>, TomlLexerError> {
        let mut tokens = Vec::new();
        let mut buffer = String::new();

        while let Some(value) = self.peek(0) {
            if value.is_alphabetic() {
                buffer.push(self.consume().unwrap());
                while self
                    .peek(0)
                    .is_some_and(|e| e.is_alphanumeric() || e == '_' || e == '-')
                {
                    buffer.push(self.consume().unwrap());
                }

                if buffer == "true" {
                    tokens.push(TomlToken::Boolean(true));
                    buffer.clear();
                    continue;
                } else if buffer == "false" {
                    tokens.push(TomlToken::Boolean(false));
                    buffer.clear();
                    continue;
                }

                tokens.push(TomlToken::Identifier(buffer.clone()));
                buffer.clear();
                continue;
            }
            if value == '#' {
                self.consume();
                while self.peek(0).is_some_and(|e| e != '\n') {
                    self.consume();
                }
                continue;
            }
            if value == ',' {
                self.consume();
                tokens.push(TomlToken::Comma);
                continue;
            }
            if value == '\n' {
                self.consume();
                tokens.push(TomlToken::NewLine);
                continue;
            }
            if value == '\"' {
                self.consume();
                loop {
                    while self
                        .peek(0)
                        .is_some_and(|e| e != '\\' && e != '\"' && e != '\n')
                    {
                        buffer.push(self.consume().unwrap());
                    }

                    if self.peek(0).is_some_and(|e| e == '\"')
                        && self.peek(1).is_some_and(|e| e == '\"')
                    {
                        self.consume();
                        self.consume();

                        if self.peek(0).is_some_and(|e| e == '\n') {
                            self.consume();
                        }
                        loop {
                            while self.peek(0).is_some_and(|e| e != '\\' && e != '\"') {
                                buffer.push(self.consume().unwrap());
                            }

                            if self.peek(0).is_some_and(|e| e == '\"')
                                && self.peek(1).is_some_and(|e| e == '\"')
                                && self.peek(2).is_some_and(|e| e == '\"')
                            {
                                self.consume();
                                self.consume();
                                self.consume();
                                break;
                            }

                            if self.peek(0).is_some_and(|e| e == '\\') {
                                self.consume();
                                match self.consume().ok_or(TomlLexerError::EndOfBuffer)? {
                                    'b' => buffer.push('\u{08}'),
                                    't' => buffer.push('\u{09}'),
                                    'n' => buffer.push('\u{0A}'),
                                    'f' => buffer.push('\u{0C}'),
                                    'r' => buffer.push('\u{0D}'),
                                    '\"' => buffer.push('\u{22}'),
                                    '\\' => buffer.push('\u{5C}'),
                                    _ => {
                                        while self
                                            .peek(0)
                                            .is_some_and(|e| e.is_whitespace() || e == '\n')
                                        {
                                            self.consume();
                                        }
                                    }
                                }
                                continue;
                            }

                            if self.peek(0).is_some_and(|e| e == '\"')
                                || self.peek(1).is_some_and(|e| e == '\"')
                            {
                                if self.peek(0).is_some_and(|e| e == '\"')
                                    && self.peek(1).is_some_and(|e| e == '\"')
                                {
                                    buffer.push(self.consume().unwrap());
                                }
                                buffer.push(self.consume().unwrap());
                                continue;
                            }
                            break;
                        }
                        break;
                    }

                    if self.peek(0).is_some_and(|e| e == '\n') {
                        return Err(TomlLexerError::ExpectedEndDoubleQuote(Some('\n')));
                    }

                    if self.peek(0).is_some_and(|e| e == '\\') {
                        self.consume();
                        match self.consume().ok_or(TomlLexerError::EndOfBuffer)? {
                            'b' => buffer.push('\u{08}'),
                            't' => buffer.push('\u{09}'),
                            'n' => buffer.push('\u{0A}'),
                            'f' => buffer.push('\u{0C}'),
                            'r' => buffer.push('\u{0D}'),
                            '"' => buffer.push('\u{22}'),
                            '\\' => buffer.push('\u{5C}'),
                            invalid_escape => {
                                return Err(TomlLexerError::InvalidEscapeSequence(format!(
                                    "\\{}",
                                    invalid_escape
                                )))
                            }
                        }
                        continue;
                    }
                    if self.peek(0).is_some_and(|e| e == '\"') {
                        self.consume();
                    } else {
                        return Err(TomlLexerError::ExpectedEndDoubleQuote(self.peek(0)));
                    }
                    break;
                }

                tokens.push(TomlToken::String(buffer.clone()));
                buffer.clear();
                continue;
            }
            if value == '\'' {
                self.consume();
                loop {
                    while self.peek(0).is_some_and(|e| e != '\'') {
                        buffer.push(self.consume().unwrap());
                    }

                    if self.peek(0).is_some_and(|e| e == '\'')
                        && self.peek(1).is_some_and(|e| e == '\'')
                    {
                        self.consume();
                        self.consume();

                        if self.peek(0).is_some_and(|e| e == '\n') {
                            self.consume();
                        }
                        loop {
                            while self.peek(0).is_some_and(|e| e != '\'') {
                                buffer.push(self.consume().unwrap());
                            }

                            if self.peek(0).is_some_and(|e| e == '\'')
                                && self.peek(1).is_some_and(|e| e == '\'')
                                && self.peek(2).is_some_and(|e| e == '\'')
                            {
                                self.consume();
                                self.consume();
                                self.consume();
                                break;
                            }
                            if self.peek(0).is_some_and(|e| e == '\'')
                                || self.peek(1).is_some_and(|e| e == '\'')
                            {
                                if self.peek(0).is_some_and(|e| e == '\'')
                                    && self.peek(1).is_some_and(|e| e == '\'')
                                {
                                    buffer.push(self.consume().unwrap());
                                }
                                buffer.push(self.consume().unwrap());
                                continue;
                            }
                            break;
                        }
                        break;
                    }
                    if self.peek(0).is_some_and(|e| e == '\n') {
                        return Err(TomlLexerError::ExpectedEndSingleQuote(Some('\n')));
                    }
                    if self.peek(0).is_some_and(|e| e == '\'') {
                        self.consume();
                    } else {
                        return Err(TomlLexerError::ExpectedEndSingleQuote(self.peek(0)));
                    }
                    break;
                }
                tokens.push(TomlToken::String(buffer.clone()));
                buffer.clear();
                continue;
            }
            if value.is_digit(10) || value == '+' || value == '-' {
                if value == '0' && self.peek(1).is_some_and(|e| e == 'x') {
                    self.consume();
                    self.consume();

                    while self.peek(0).is_some_and(|e| e.is_digit(16) || e == '_') {
                        if self.peek(0).is_some_and(|e| e == '_') {
                            self.consume();
                            continue;
                        }
                        buffer.push(self.consume().unwrap());
                    }

                    tokens.push(TomlToken::Interger(
                        i64::from_str_radix(&buffer, 16)
                            .map_err(TomlLexerError::InvalidInterger)?,
                    ));
                    buffer.clear();
                    continue;
                }
                if value == '0' && self.peek(1).is_some_and(|e| e == 'o') {
                    self.consume();
                    self.consume();

                    while self.peek(0).is_some_and(|e| e.is_digit(8)) {
                        if self.peek(0).is_some_and(|e| e == '_') {
                            self.consume();
                            continue;
                        }

                        buffer.push(self.consume().unwrap());
                    }

                    tokens.push(TomlToken::Interger(
                        i64::from_str_radix(&buffer, 8).map_err(TomlLexerError::InvalidInterger)?,
                    ));
                    buffer.clear();
                    continue;
                }
                if value == '0' && self.peek(1).is_some_and(|e| e == 'b') {
                    self.consume();
                    self.consume();

                    while self.peek(0).is_some_and(|e| e.is_digit(2)) {
                        if self.peek(0).is_some_and(|e| e == '_') {
                            self.consume();
                            continue;
                        }

                        buffer.push(self.consume().unwrap());
                    }

                    tokens.push(TomlToken::Interger(
                        i64::from_str_radix(&buffer, 2).map_err(TomlLexerError::InvalidInterger)?,
                    ));
                    buffer.clear();
                    continue;
                }

                buffer.push(self.consume().unwrap());

                while self.peek(0).is_some_and(|e| e.is_digit(10) || e == '_') {
                    if self.peek(0).is_some_and(|e| e == '_') {
                        self.consume();
                        continue;
                    }

                    buffer.push(self.consume().unwrap());
                }
                if self.peek(0).is_some_and(|e| e == '.') {
                    return Err(TomlLexerError::FloatNumberNotSupport);
                }
                tokens.push(TomlToken::Interger(
                    buffer
                        .parse::<i64>()
                        .map_err(TomlLexerError::InvalidInterger)?,
                ));
                buffer.clear();
                continue;
            }
            if value == '{' {
                self.consume();
                tokens.push(TomlToken::LCurly);
                continue;
            }
            if value == '}' {
                self.consume();
                tokens.push(TomlToken::RCurly);
                continue;
            }
            if value == '[' {
                self.consume();
                tokens.push(TomlToken::LBracket);
                continue;
            }
            if value == ']' {
                self.consume();
                tokens.push(TomlToken::RBracket);
                continue;
            }
            if value == '=' {
                self.consume();
                tokens.push(TomlToken::Equal);
                continue;
            }
            if value == '.' {
                self.consume();
                tokens.push(TomlToken::Dot);
                continue;
            }
            if value.is_whitespace() {
                self.consume();
                continue;
            }
            buffer.push(self.consume().unwrap());
            return Err(TomlLexerError::InvalidToken(buffer));
        }

        return Ok(tokens);
    }
}
