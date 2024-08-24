use core::slice::from_raw_parts_mut;

use conquer_once::spin::OnceCell;
use spin::mutex::Mutex;
use uefi::proto::console::gop::{ModeInfo, PixelFormat};

use common::boot::BootInformation;

pub static DRIVER: OnceCell<Mutex<Graphic>> = OnceCell::uninit();

pub struct Graphic {
    mode: ModeInfo,
    frame_buffer: &'static mut [u32],
}

impl Graphic {
    pub fn new(mode: ModeInfo, frame_buffer: &'static mut [u32]) -> Self {
        Self { mode, frame_buffer }
    }

    pub fn plot(&mut self, x: usize, y: usize, color: u32) {
        let (width, height) = self.mode.resolution();
        if x > width || y > height {
            return;
        }

        match self.mode.pixel_format() {
            PixelFormat::Rgb => {
                self.frame_buffer[y * width + x] = color << 8;
            }
            PixelFormat::Bgr => {
                self.frame_buffer[y * width + x] = color;
            }
            PixelFormat::Bitmask => {}
            PixelFormat::BltOnly => {}
        }
    }

    pub fn get_res(&self) -> (usize, usize) {
        return self.mode.resolution();
    }
}

pub fn init(bootinfo: &BootInformation) {
    DRIVER.init_once(|| unsafe {
        let (width, height) = bootinfo.gop_mode.info().resolution();
        let buffer = from_raw_parts_mut(bootinfo.framebuffer, width * height * 4);
        Mutex::new(Graphic::new(bootinfo.gop_mode.info().clone(), buffer))
    });
}
