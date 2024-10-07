use core::{
    fmt::{Display, Formatter, Result},
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll},
};

use alloc::{format, string::String};
use spin::RwLock;

use crate::utils::circular_ring_buffer::CircularRingBuffer;

pub static LOGGER: Logger = Logger::new();
const LOG_BUFFER_SIZE: usize = 1024;
const LOG_SUBSCRIBERS_SIZE: usize = 16;

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
    subscribers: RwLock<[Option<LoggerSubscriber>; LOG_SUBSCRIBERS_SIZE]>,
    subscribers_index: AtomicUsize,
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
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        cx.waker().clone().wake();
        match self.buffer.read() {
            Some(log) => Poll::Ready(log),
            None => Poll::Pending,
        }
    }
}

impl Logger {
    pub const fn new() -> Self {
        Self {
            main_buffer: CircularRingBuffer::new(),
            subscribers: RwLock::new([const { None }; LOG_SUBSCRIBERS_SIZE]),
            subscribers_index: AtomicUsize::new(0),
        }
    }

    pub fn write(&self, log: Log) {
        self.main_buffer.write(log);
    }

    pub fn add_target(&self, display: fn(&str)) {
        self.subscribers.write()[self.subscribers_index.load(Ordering::Acquire)] =
            Some(LoggerSubscriber { display });
        self.subscribers_index.fetch_add(1, Ordering::Release);
    }

    fn log_msg(&self, msg: Log) {
        let msg = format!("{}", msg);
        for subscriber in self.subscribers.read().iter() {
            let subscriber = match subscriber {
                Some(subscriber) => subscriber,
                None => break,
            };
            (subscriber.display)(&msg);
        }
    }

    pub async fn log_async(&self) {
        loop {
            let msg = LoggerAsync::new(&self.main_buffer).await;
            self.log_msg(msg);
        }
    }

    pub fn flush_all(&self) {
        while let Some(msg) = self.main_buffer.read() {
            self.log_msg(msg);
        }
    }
}
