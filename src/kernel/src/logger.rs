use core::fmt::{Arguments, Display, Formatter, Result, Write};

use bootbridge::BootBridge;
use c_enum::c_enum;
use static_log::StaticLog;

mod static_log;

pub static LOGGER: MainLogger = MainLogger::new();
const BUFFER_SIZE: usize = 0x2000;

#[macro_export]
macro_rules! log {
    ($level:ident, $($arg:tt)*) => {{
        $crate::logger::LOGGER.write($crate::logger::LogLevel::$level, format_args!("{}\n", format_args!($($arg)*)));
    }};
}

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

impl Display for LogLevel {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
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

struct CallbackFormatter<C: FnMut(&str)> {
    callback: C,
}

impl<C: FnMut(&str)> CallbackFormatter<C> {
    pub fn new(callback: C) -> Self {
        Self { callback }
    }
}

impl<C: FnMut(&str)> Write for CallbackFormatter<C> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        (self.callback)(s);
        Ok(())
    }
}

pub struct MainLogger {
    logger: StaticLog<BUFFER_SIZE>,
    level: LogLevel,
}

impl MainLogger {
    pub const fn new() -> Self {
        Self {
            logger: StaticLog::new(),
            level: LogLevel::Trace,
        }
    }

    pub unsafe fn set_level(&mut self, level: u64) {
        self.level = LogLevel(level);
    }

    pub fn write(&self, level: LogLevel, formatter: Arguments) {
        if level < self.level {
            return;
        }
        self.logger.write_log(&formatter, level);
    }

    pub fn flush_all(&self, displays: &[fn(&str)]) {
        while let Some(losts) = self.logger.read(CallbackFormatter::new(|s| {
            displays.iter().for_each(|d| (d)(s));
        })) {
            if losts == 0 {
                continue;
            }
            let _ = CallbackFormatter::new(|s| {
                displays.iter().for_each(|d| (d)(s));
            })
            .write_fmt(format_args!(
                "\x1b[93mWARNING\x1b[0m: Could not recover some logs, lost {losts} bytes"
            ));
        }
    }
}

pub fn init(boot_bridge: &BootBridge) {
    // Altho this looks so fucking unsafe but i can asure you it's safe
    // Because the init function is only being called when the kernel is initialize (guarantee by my
    // stupidity)
    unsafe {
        #[allow(invalid_reference_casting)]
        (&mut *(&LOGGER as *const MainLogger as *mut MainLogger))
            .set_level(boot_bridge.log_level());
    };
    log!(Trace, "Logging start");
}
