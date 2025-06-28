#![no_std]

use core::fmt::{Arguments, Display, Formatter};

use c_enum::c_enum;
use conquer_once::spin::OnceCell;

#[macro_export]
macro_rules! log {
    ($level:ident, $($arg:tt)*) => {{
        $crate::log_message(module_path!(), $crate::LogLevel::$level, format_args!("{}\n", format_args!($($arg)*)));
    }};
}

static LOGGER_BACKEND: OnceCell<&'static dyn LoggerBackend> = OnceCell::uninit();

c_enum! {
    #[derive(Debug)]
    pub enum LogLevel: u64 {
        Trace       = 1
        Debug       = 2
        Info        = 3
        Warning     = 4
        Error       = 5
        Critical    = 6
    }
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Debug
    }
}

impl From<u64> for LogLevel {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl Display for LogLevel {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), core::fmt::Error> {
        match *self {
            Self::Debug => write!(f, "\x1b[92mDEBUG\x1b[0m"),
            Self::Info => write!(f, "\x1b[92mINFO\x1b[0m"),
            Self::Trace => write!(f, "\x1b[94mTRACE\x1b[0m"),
            Self::Error => write!(f, "\x1b[91mERROR\x1b[0m"),
            Self::Warning => write!(f, "\x1b[93mWARNING\x1b[0m"),
            Self::Critical => write!(f, "\x1b[31mCRITICAL\x1b[0m"),
            _ => unreachable!(),
        }
    }
}

pub trait LoggerBackend: Sync {
    fn log(&self, module_path: &'static str, level: LogLevel, formatter: Arguments);
}

pub fn set_logger(backend: &'static dyn LoggerBackend) {
    LOGGER_BACKEND.init_once(|| backend);
}

pub fn get_logger() -> Option<&'static dyn LoggerBackend> {
    LOGGER_BACKEND.get().copied()
}

pub fn log_message(module_path: &'static str, level: LogLevel, formatter: Arguments) {
    if let Some(logger) = LOGGER_BACKEND.get() {
        logger.log(module_path, level, formatter);
    }
}
