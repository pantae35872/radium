use core::fmt::{Display, Formatter, Result};

use alloc::{format, string::String};
use spin::mutex::Mutex;

pub static LOGGER: Mutex<Logger> = Mutex::new(Logger::new());
const LOG_BUFFER_SIZE: usize = 1024;
const LOG_TARGET_SIZE: usize = 16;

#[macro_export]
macro_rules! log {
    ($level:ident, $fmt:expr $(, $args:expr)*) => {
        {
            let message = alloc::format!($fmt, $($args),*);
            $crate::logger::LOGGER.lock().write($crate::logger::Log::new($crate::logger::LogLevel::$level, message));
            $crate::logger::LOGGER.lock().update();
        }
    };
}

pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

pub struct Log {
    level: LogLevel,
    message: String,
}

struct LoggerTarget {
    display: fn(msg: &str),
    current: usize,
}

pub struct Logger {
    buffer: [Option<Log>; LOG_BUFFER_SIZE],
    targets: [Option<LoggerTarget>; LOG_TARGET_SIZE],
    head: usize,
    tail: usize,
    logger_index: usize,
}

impl Log {
    pub fn new(level: LogLevel, message: String) -> Self {
        Self { level, message }
    }
}

impl Display for Log {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{}: {}", self.level, self.message)
    }
}

impl Display for LogLevel {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Self::Debug => write!(f, "\x1b[92mDEBUG\x1b[0m"),
            Self::Info => write!(f, "\x1b[92mINFO\x1b[0m"),
            Self::Trace => write!(f, "\x1b[94mTRACE\x1b[0m"),
            Self::Error => write!(f, "\x1b[91mERROR\x1b[0m"),
            Self::Warning => write!(f, "\x1b[93mWARNING\x1b[0m"),
            Self::Critical => write!(f, "\x1b[31mCRITICAL\x1b[0m"),
        }
    }
}

impl Logger {
    pub const fn new() -> Self {
        Self {
            targets: [const { None }; LOG_TARGET_SIZE],
            buffer: [const { None }; LOG_BUFFER_SIZE],
            head: 0,
            tail: 0,
            logger_index: 0,
        }
    }

    pub fn write(&mut self, log: Log) {
        self.buffer[self.head] = Some(log);
        self.head = (self.head + 1) % LOG_BUFFER_SIZE;

        if self.head == self.tail {
            self.tail = (self.tail + 1) % LOG_BUFFER_SIZE;
        }
    }

    pub fn add_target(&mut self, display: fn(&str)) {
        self.targets[self.logger_index] = Some(LoggerTarget {
            display,
            current: 0,
        });
        self.logger_index = (self.logger_index + 1) % LOG_TARGET_SIZE;
        self.update();
    }

    pub fn update(&mut self) {
        for target in self.targets.iter_mut() {
            let target = match target {
                Some(target) => target,
                None => continue,
            };
            while target.current != self.head {
                if let Some(ref message) = self.buffer[target.current] {
                    (target.display)(&format!("{message}"));
                }
                target.current = (target.current + 1) % LOG_BUFFER_SIZE;
            }
        }
    }
}
