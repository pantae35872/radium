use core::fmt;
use core::fmt::{Arguments, Write};

use crate::graphics::color::Color;
use crate::logger::LOGGER;
use crate::BootInformation;
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

pub fn init(bootinfo: &BootInformation, foreground_color: Color, background: Color) {
    DRIVER.init_once(|| Mutex::new(Print::new(bootinfo, foreground_color, background)));
    LOGGER.lock().add_target(|msg| {
        println!("{msg}");
    });
}

pub struct Print {
    renderer: TtfRenderer,
}

impl Print {
    pub fn new(bootinfo: &BootInformation, foreground: Color, background: Color) -> Self {
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
