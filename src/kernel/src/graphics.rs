use bit_field::BitField;
use color::Color;
use conquer_once::spin::OnceCell;
use spin::mutex::Mutex;
use uefi::proto::console::gop::{ModeInfo, PixelFormat};

use common::boot::BootInformation;

use crate::{log, memory::memory_controller};

pub mod color;

pub static DRIVER: OnceCell<Mutex<Graphic>> = OnceCell::uninit();

pub struct Graphic {
    mode: ModeInfo,
    plot_fn: for<'a> fn(&'a mut Self, color: Color, y: usize, x: usize),
    get_pixel_fn: for<'a> fn(&'a Self, x: usize, y: usize) -> Color,
    frame_buffer: &'static mut [u32],
}

pub const BACKGROUND_COLOR: Color = Color::new(27, 38, 59);

impl Graphic {
    pub fn new(mode: ModeInfo, frame_buffer: &'static mut [u32]) -> Self {
        let (horizontal, vertical) = mode.resolution();
        log!(Info, "Graphic resolution {}x{}", horizontal, vertical);
        let plot_fn = match mode.pixel_format() {
            PixelFormat::Rgb => Self::plot_rgb,
            PixelFormat::Bgr => Self::plot_bgr,
            PixelFormat::Bitmask => Self::plot_bitmask,
            PixelFormat::BltOnly => unimplemented!("Not support"),
        };
        let get_pixel_fn = match mode.pixel_format() {
            PixelFormat::Rgb => Self::get_pixel_rgb,
            PixelFormat::Bgr => Self::get_pixel_bgr,
            PixelFormat::Bitmask => Self::get_pixel_bitmask,
            PixelFormat::BltOnly => unimplemented!("Not support"),
        };
        let mut va = Self {
            mode,
            plot_fn,
            get_pixel_fn,
            frame_buffer,
        };
        for y in 0..vertical {
            for x in 0..horizontal {
                va.plot(x, y, BACKGROUND_COLOR);
            }
        }
        va
    }

    pub fn plot(&mut self, x: usize, y: usize, color: Color) {
        let (width, height) = self.mode.resolution();
        if x >= width || y >= height {
            return;
        }

        (self.plot_fn)(self, color, y, x);
    }

    pub fn scroll_up(&mut self, scroll_amount: usize) {
        let (width, height) = self.mode.resolution();

        for y in 0..(height - scroll_amount) {
            for x in 0..width {
                let src_y = y + scroll_amount;
                let color = (self.get_pixel_fn)(self, src_y, x);
                (self.plot_fn)(self, color, y, x);
            }
        }

        for y in (height - scroll_amount)..height {
            for x in 0..width {
                (self.plot_fn)(self, BACKGROUND_COLOR, y, x);
            }
        }
    }

    fn plot_rgb(&mut self, color: Color, y: usize, x: usize) {
        self.frame_buffer[y * self.mode.stride() + x] = color.as_u32() << 8;
    }

    fn plot_bgr(&mut self, color: Color, y: usize, x: usize) {
        self.frame_buffer[y * self.mode.stride() + x] = color.as_u32();
    }

    fn plot_bitmask(&mut self, color: Color, y: usize, x: usize) {
        match self.mode.pixel_bitmask() {
            Some(bitmask) => {
                self.frame_buffer[y * self.mode.stride() + x] =
                    color.apply_bitmask(bitmask.red, bitmask.green, bitmask.blue);
            }
            None => {}
        }
    }

    fn get_pixel_rgb(&self, y: usize, x: usize) -> Color {
        let color = self.frame_buffer[y * self.mode.stride() + x];
        return Color::new(
            color.get_bits(24..32) as u8,
            color.get_bits(16..24) as u8,
            color.get_bits(8..16) as u8,
        );
    }

    fn get_pixel_bgr(&self, y: usize, x: usize) -> Color {
        let color = self.frame_buffer[y * self.mode.stride() + x];
        return Color::new(
            color.get_bits(16..24) as u8,
            color.get_bits(8..16) as u8,
            color.get_bits(0..8) as u8,
        );
    }

    fn get_pixel_bitmask(&self, y: usize, x: usize) -> Color {
        match self.mode.pixel_bitmask() {
            Some(bitmask) => {
                let color = self.frame_buffer[y * self.mode.stride() + x];
                let red = color.get_bits(
                    (bitmask.red.trailing_zeros() - 8) as usize
                        ..bitmask.red.trailing_zeros() as usize,
                );
                let green = color.get_bits(
                    (bitmask.green.trailing_zeros() - 8) as usize
                        ..bitmask.green.trailing_zeros() as usize,
                );
                let blue = color.get_bits(
                    (bitmask.blue.trailing_zeros() - 8) as usize
                        ..bitmask.blue.trailing_zeros() as usize,
                );
                return Color::new(red as u8, green as u8, blue as u8);
            }
            None => Color::new(0, 0, 0),
        }
    }

    pub fn get_res(&self) -> (usize, usize) {
        return self.mode.resolution();
    }
}

pub fn init(bootinfo: &BootInformation) {
    log!(Info, "Initializing graphic");
    DRIVER.init_once(|| {
        memory_controller().lock().ident_map(
            bootinfo.framebuffer_size() as u64,
            bootinfo
                .framebuffer_addr()
                .expect("Frame buffer has been already aquired"),
        );
        Mutex::new(Graphic::new(
            *bootinfo.gop_mode_info(),
            bootinfo
                .framebuffer()
                .expect("Failed to aquire framebuffer from bootinfo it already been taken"),
        ))
    });
}
