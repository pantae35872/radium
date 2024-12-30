use bit_field::BitField;
use color::Color;
use conquer_once::spin::OnceCell;
use spin::mutex::Mutex;
use uefi::proto::console::gop::{ModeInfo, PixelFormat};

use common::boot::BootInformation;

use crate::{
    log,
    memory::{memory_controller, paging::EntryFlags, virt_addr_alloc},
};

pub mod color;

pub static DRIVER: OnceCell<Mutex<Graphic>> = OnceCell::uninit();

pub struct Graphic {
    mode: ModeInfo,
    plot_fn: for<'a> unsafe fn(&'a mut Self, color: Color, y: usize, x: usize),
    #[allow(unused)]
    get_pixel_fn: for<'a> fn(&'a Self, x: usize, y: usize) -> Color,
    frame_buffer: &'static mut [u32],
    real_buffer: &'static mut [u32],
    min_render_x: usize,
    min_render_y: usize,
    max_render_x: usize,
    max_render_y: usize,
    //min_glyph_render_x: usize,
    //min_glyph_render_y: usize,
    //max_glyph_render_x: usize,
    //max_glyph_render_y: usize,
}

pub const BACKGROUND_COLOR: Color = Color::new(33, 33, 33);

impl Graphic {
    pub fn new(mode: ModeInfo, frame_buffer: &'static mut [u32]) -> Self {
        let (width, height) = mode.resolution();
        log!(Info, "Graphic resolution {}x{}", width, height);
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
        let framebuffer_len = frame_buffer.len();
        let framebuffer_size = (framebuffer_len * size_of::<u32>()) as u64;
        let virt = virt_addr_alloc(framebuffer_size);
        memory_controller().lock().alloc_map(framebuffer_size, virt);
        let mut va = Self {
            mode,
            plot_fn,
            get_pixel_fn,
            real_buffer: frame_buffer,
            frame_buffer: unsafe {
                core::slice::from_raw_parts_mut(virt as *mut u32, framebuffer_len)
            },
            min_render_x: width - 1,
            min_render_y: height - 1,
            max_render_x: 0,
            max_render_y: 0,
            /*min_glyph_render_x: 0,
            min_glyph_render_y: 0,
            max_glyph_render_x: 0,
            max_glyph_render_y: 0,*/
        };
        for y in 0..height {
            for x in 0..width {
                va.plot(x, y, BACKGROUND_COLOR);
            }
        }
        va.swap();
        va
    }

    /// Performs a backbuffer swap
    pub fn swap(&mut self) {
        let (width, height) = self.mode.resolution();
        let min_pos = self.min_render_y * self.mode.stride() + self.min_render_x;
        let max_pos = self.max_render_y * self.mode.stride() + self.max_render_x;
        self.real_buffer[min_pos..max_pos].copy_from_slice(&self.frame_buffer[min_pos..max_pos]);
        self.min_render_x = width - 1;
        self.min_render_y = height - 1;
        self.max_render_x = 0;
        self.max_render_y = 0;
    }

    pub fn plot(&mut self, x: usize, y: usize, color: Color) {
        let (width, height) = self.mode.resolution();
        if x >= width || y >= height {
            return;
        }

        self.min_render_x = self.min_render_x.min(x);
        self.min_render_y = self.min_render_y.min(y);
        self.max_render_x = self.max_render_x.max(x);
        self.max_render_y = self.max_render_y.max(y);

        unsafe {
            (self.plot_fn)(self, color, y, x);
        }
    }

    pub fn scroll_up(&mut self, scroll_amount: usize) {
        let (width, height) = self.mode.resolution();
        self.min_render_x = 0;
        self.min_render_y = 0;
        self.max_render_x = width - 1;
        self.max_render_y = height - 1;

        unsafe {
            let scroll = &self.frame_buffer[(self.mode.stride() * scroll_amount)..];
            let scroll_len = scroll.len();
            let scroll = &scroll[0] as *const u32;
            core::ptr::copy(scroll, &mut self.frame_buffer[0] as *mut u32, scroll_len);
        }

        self.frame_buffer[(self.mode.stride() * (height - scroll_amount))..].fill(
            match self.mode.pixel_format() {
                PixelFormat::Rgb => BACKGROUND_COLOR.as_u32() << 8,
                PixelFormat::Bgr => BACKGROUND_COLOR.as_u32(),
                PixelFormat::Bitmask => match self.mode.pixel_bitmask() {
                    Some(bitmask) => {
                        BACKGROUND_COLOR.apply_bitmask(bitmask.red, bitmask.green, bitmask.blue)
                    }
                    None => BACKGROUND_COLOR.as_u32(), // Assumes bgr
                },
                PixelFormat::BltOnly => unimplemented!("Not support"),
            },
        );
    }

    unsafe fn plot_rgb(&mut self, color: Color, y: usize, x: usize) {
        unsafe {
            *self
                .frame_buffer
                .get_unchecked_mut(y * self.mode.stride() + x) = color.as_u32() << 8;
        }
    }

    unsafe fn plot_bgr(&mut self, color: Color, y: usize, x: usize) {
        unsafe {
            *self
                .frame_buffer
                .get_unchecked_mut(y * self.mode.stride() + x) = color.as_u32();
        }
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
    log!(Trace, "Initializing graphic");
    DRIVER.init_once(|| {
        memory_controller().lock().ident_map(
            bootinfo.framebuffer_size() as u64,
            bootinfo
                .framebuffer_addr()
                .expect("Frame buffer has been already aquired"),
            EntryFlags::WRITABLE | EntryFlags::NO_CACHE | EntryFlags::PRESENT,
        );
        Mutex::new(Graphic::new(
            *bootinfo.gop_mode_info(),
            bootinfo
                .framebuffer()
                .expect("Failed to aquire framebuffer from bootinfo it already been taken"),
        ))
    });
}
