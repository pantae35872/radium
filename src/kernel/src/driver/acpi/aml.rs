use alloc::boxed::Box;
use namespace::{AmlName, Namespace};
use parser::{Parser, Propagate, term_object::term_list};
use sentinel::log;

pub mod namespace;
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

pub trait AmlHandle: Send + Sync {
    fn write_debug(&self, value: &str);
}

pub struct AmlContext {
    pub namespace: Namespace,
    current_scope: AmlName,
    handle: Box<dyn AmlHandle>,
}

impl AmlContext {
    pub fn new(handle: impl AmlHandle + 'static) -> Self {
        Self { namespace: Namespace::new(), current_scope: AmlName::root(), handle: Box::new(handle) }
    }
}

pub fn init<'a, 'c>(code: &'a [u8], context: &'c mut AmlContext) -> Option<(&'a [u8], AmlError)>
where
    'c: 'a,
{
    term_list(code.len() as u32)
        .parse(code, context)
        .map(|_| ())
        .map_err(|(left_over, _context, err)| {
            (
                left_over,
                match err {
                    Propagate::AmlError(err) => err,
                    _ => panic!("Aml does not return an error"),
                },
            )
        })
        .err()
}

struct TestHandle;

impl AmlHandle for TestHandle {
    fn write_debug(&self, value: &str) {
        log!(Trace, "Aml log : {value}");
    }
}

impl Into<Propagate> for AmlError {
    fn into(self) -> Propagate {
        Propagate::AmlError(self)
    }
}
