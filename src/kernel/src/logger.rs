use core::{
    cell::SyncUnsafeCell,
    fmt::{Arguments, Write},
};

use sentinel::{log, set_logger, LogLevel, LoggerBackend};
use static_log::StaticLog;

use crate::{
    initialization_context::{InitializationContext, Stage0},
    initialize_guard, print, serial_print,
    smp::{cpu_local, cpu_local_avaiable},
};

mod static_log;

pub static LOGGER: MainLogger = MainLogger::new();
const BUFFER_SIZE: usize = 0x4000;

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
    /// SAFETY: This is sync because this is our kernel does not have multi threaded
    /// initialization and the logger instance is only initialize once across all cores
    level: SyncUnsafeCell<LogLevel>,
}

impl MainLogger {
    pub const fn new() -> Self {
        Self {
            logger: StaticLog::new(),
            level: SyncUnsafeCell::new(LogLevel::Trace),
        }
    }

    /// Set the log level unatomically 
    ///
    /// # Safety
    /// the caller must ensure that this is only being called on kernel initialization
    pub unsafe fn set_level(&self, level: u64) {
        unsafe {
            *self.level.get() = LogLevel::from(level);
        }
    }

    pub fn write(&self, level: LogLevel, formatter: Arguments) {
        // SAFETY: This is safe because the function that mutates the level only being called on
        // initialization (gureentee by unsafe)
        if level < unsafe { *self.level.get() } {
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

    pub fn flush_select(&self) {
        self.flush_all(if crate::print::DRIVER.get().is_none() {
            log!(
                Warning,
                "Screen print not avaiable logging into serial ports"
            );
            &[|s| serial_print!("{s}")]
        } else if crate::TESTING {
            &[|s| serial_print!("{s}")]
        } else {
            &[|s| serial_print!("{s}"), |s| print!("{s}")]
        });
    }
}

impl Default for MainLogger {
    fn default() -> Self {
        Self::new()
    }
}

impl LoggerBackend for MainLogger {
    fn log(&self, module_path: &'static str, level: LogLevel, formatter: Arguments) {
        if cpu_local_avaiable() {
            if cpu_local().is_in_isr {
                self.write(
                    level,
                    format_args!(
                        "<- [\x1b[93m{module_path}\x1b[0m] [C {core} : \x1b[94mIN ISR\x1b[0m] : {formatter}",
                        core = cpu_local().core_id().id(),
                    ),
                );
            } else {
                self.write(
                    level,
                    format_args!(
                        "<- [\x1b[93m{module_path}\x1b[0m] [C {core} : T {thread}] : {formatter}",
                        core = cpu_local().core_id().id(),
                        thread = cpu_local().current_thread_id(),
                    ),
                );
            }
        } else {
            self.write(
                level,
                format_args!("<- [\x1b[93m{module_path}\x1b[0m] [C ? : T ?] : {formatter}",),
            );
        };
    }
}

pub fn init(ctx: &InitializationContext<Stage0>) {
    initialize_guard!();
    // SAFETY: This is safe because the above interrupt guard
    unsafe {
        LOGGER.set_level(ctx.context().boot_bridge().log_level());
    };
    set_logger(&LOGGER);
    log!(Trace, "Logging start");
}
