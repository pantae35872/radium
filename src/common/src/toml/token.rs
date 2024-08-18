use core::fmt::Display;

use alloc::string::String;

#[derive(Debug)]
pub enum TomlToken {
    String(String),
    Interger(i64),
    Boolean(bool),
    LBracket,
    RBracket,
    LCurly,
    RCurly,
    Equal,
    NewLine,
    Comma,
    Dot,
    Identifier(String),
}

impl Display for TomlToken {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::String(string) => {
                write!(f, "String literial token with value: {}", string)
            }
            Self::Interger(value) => write!(f, "Interger token with value: {}", value),
            Self::Boolean(value) => write!(f, "Boolean token with value: {}", value),
            Self::LBracket => write!(f, "Left Bracket token"),
            Self::RBracket => write!(f, "Right Bracket token"),
            Self::LCurly => write!(f, "Left curly"),
            Self::RCurly => write!(f, "Right curly"),
            Self::Equal => write!(f, "Equal token"),
            Self::Comma => write!(f, "Comma token"),
            Self::Dot => write!(f, "Dot token"),
            Self::Identifier(value) => write!(f, "Identifier token with value: {}", value),
            Self::NewLine => write!(f, "New line token"),
        }
    }
}
