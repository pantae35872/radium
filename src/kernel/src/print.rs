use core::fmt;
use core::fmt::{Arguments, Write};

use crate::graphics::color::Color;
use crate::log;
use crate::logger::LOGGER;
use bootbridge::BootBridge;
use conquer_once::spin::OnceCell;
use spin::Mutex;
use x86_64::instructions::interrupts;

use self::ttf_renderer::TtfRenderer;

pub mod ttf_renderer;

pub static DRIVER: OnceCell<Mutex<Print>> = OnceCell::uninit();

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::print::_print(format_args!($($arg)*))
    };
}

pub fn _print(args: Arguments) {
    interrupts::without_interrupts(|| {
        if let Some(driver) = DRIVER.get() {
            driver.lock().write_fmt(args).unwrap();
        } else {
            panic!("Use of uninitialize driver (Print driver)");
        }
    });
}

#[macro_export]
macro_rules! println {
    ($($arg:tt)*) => {{
        $crate::print!("{}\n", format_args!($($arg)*));
    }};
}

pub fn init(bootinfo: &BootBridge, foreground_color: Color, background: Color) {
    log!(Trace, "Initializing text output");
    DRIVER.init_once(|| Mutex::new(Print::new(bootinfo, foreground_color, background)));
}

pub struct Print {
    renderer: TtfRenderer,
}

impl Print {
    pub fn new(bootinfo: &BootBridge, foreground: Color, background: Color) -> Self {
        return Self {
            renderer: TtfRenderer::new(bootinfo, foreground, background),
        };
    }

    pub fn set_color(&mut self, foreground: Color) {
        self.renderer.set_color(foreground);
    }

    pub fn print_str(&mut self, string: &str) {
        self.renderer.put_str(string);
    }
}

impl fmt::Write for Print {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.print_str(s);
        Ok(())
    }
}
