use color::Color;
use conquer_once::spin::OnceCell;
use spin::mutex::Mutex;
use uefi::proto::console::gop::{ModeInfo, PixelFormat};

use common::boot::BootInformation;

pub mod color;

pub static DRIVER: OnceCell<Mutex<Graphic>> = OnceCell::uninit();

pub struct Graphic {
    mode: ModeInfo,
    plot_fn: for<'a> fn(&'a mut Self, color: Color, y: usize, stride: usize, x: usize),
    frame_buffer: &'static mut [u32],
}

pub const BACKGROUND_COLOR: Color = Color::new(27, 38, 59);

impl Graphic {
    pub fn new(mode: ModeInfo, frame_buffer: &'static mut [u32]) -> Self {
        let (horizontal, vertical) = mode.resolution();
        let plot_fn = match mode.pixel_format() {
            PixelFormat::Rgb => Self::plot_rgb,
            PixelFormat::Bgr => Self::plot_bgr,
            PixelFormat::Bitmask => Self::plot_bitmask,
            PixelFormat::BltOnly => unimplemented!("Not support"),
        };
        let mut va = Self {
            mode,
            plot_fn,
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
        let stride = self.mode.stride();

        (self.plot_fn)(self, color, y, stride, x);
    }

    fn plot_rgb(&mut self, color: Color, y: usize, stride: usize, x: usize) {
        self.frame_buffer[y * stride + x] = color.as_u32() << 8;
    }

    fn plot_bgr(&mut self, color: Color, y: usize, stride: usize, x: usize) {
        self.frame_buffer[y * stride + x] = color.as_u32();
    }

    fn plot_bitmask(&mut self, color: Color, y: usize, stride: usize, x: usize) {
        match self.mode.pixel_bitmask() {
            Some(bitmask) => {
                self.frame_buffer[y * stride + x] =
                    color.apply_bitmask(bitmask.red, bitmask.green, bitmask.blue);
            }
            None => {}
        }
    }

    pub fn get_res(&self) -> (usize, usize) {
        return self.mode.resolution();
    }
}

pub fn init(bootinfo: &BootInformation) {
    DRIVER.init_once(|| {
        Mutex::new(Graphic::new(
            bootinfo.gop_mode_info().clone(),
            bootinfo
                .framebuffer()
                .expect("Failed to aquire framebuffer from bootinfo it already been taken"),
        ))
    });
}
