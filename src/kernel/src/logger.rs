use core::{
    fmt::{Display, Formatter, Result},
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use alloc::{format, string::String};

use crate::utils::circular_ring_buffer::CircularRingBuffer;

pub static LOGGER: Logger = Logger::new();
const LOG_BUFFER_SIZE: usize = 1024;
const LOG_TARGET_SIZE: usize = 16;

#[macro_export]
macro_rules! log {
    ($level:ident, $fmt:expr $(, $args:expr)*) => {
        {
            let message = alloc::format!($fmt, $($args),*);
            $crate::logger::LOGGER.write($crate::logger::Log::new($crate::logger::LogLevel::$level, message));
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

struct LoggerSubscriber {
    display: fn(msg: &str),
}

pub struct Logger {
    main_buffer: CircularRingBuffer<Log, LOG_BUFFER_SIZE>,
    subscribers: CircularRingBuffer<LoggerSubscriber, LOG_TARGET_SIZE>,
    subscribers_swap: CircularRingBuffer<LoggerSubscriber, LOG_TARGET_SIZE>,
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

struct LoggerAsync<'a> {
    buffer: &'a CircularRingBuffer<Log, LOG_BUFFER_SIZE>,
}

impl<'a> LoggerAsync<'a> {
    fn new(buffer: &'a CircularRingBuffer<Log, LOG_BUFFER_SIZE>) -> Self {
        Self { buffer }
    }
}

impl<'a> Future for LoggerAsync<'a> {
    type Output = Log;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.buffer.read() {
            Some(log) => Poll::Ready(log),
            None => Poll::Pending,
        }
    }
}

impl Logger {
    pub const fn new() -> Self {
        Self {
            subscribers: CircularRingBuffer::new(),
            main_buffer: CircularRingBuffer::new(),
            subscribers_swap: CircularRingBuffer::new(),
        }
    }

    pub fn write(&self, log: Log) {
        self.main_buffer.write(log);
    }

    pub fn add_target(&self, display: fn(&str)) {
        self.subscribers.write(LoggerSubscriber { display });
    }

    pub async fn log_async(&self) {
        loop {
            let msg = LoggerAsync::new(&self.main_buffer).await;
            let mut is_some = false;
            while let Some(subscriber) = self.subscribers_swap.read() {
                (subscriber.display)(&format!("{}", msg));
                self.subscribers.write(subscriber);
                is_some = true;
            }
            if !is_some {
                while let Some(subscriber) = self.subscribers.read() {
                    (subscriber.display)(&format!("{}", msg));
                    self.subscribers_swap.write(subscriber);
                }
            }
        }
    }
}
