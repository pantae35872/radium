use namespace::{AmlName, Namespace};
use parser::Propagate;

mod namespace;
mod parser;

#[derive(Debug, PartialEq, Eq)]
pub enum AmlError {
    /// Trying to normalize an aml name with an invalid prefixes
    NormalizingInvalidName,
    /// Trying to convert name component into name seg, but the component is not a name seg
    NotANameSeg,
    LevelDoesNotExists {
        path: AmlName,
    },
    PathIsNotNormalize,
    ParserError,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AmlValue {}

#[derive(Debug)]
struct AmlContext {
    pub namespace: Namespace,
    current_scope: AmlName,
}

impl AmlContext {
    pub fn test_context() -> Self {
        Self {
            namespace: Namespace::new(),
            current_scope: AmlName::root(),
        }
    }
}

impl Into<Propagate> for AmlError {
    fn into(self) -> Propagate {
        Propagate::AmlError(self)
    }
}
