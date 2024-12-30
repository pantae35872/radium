use alloc::vec::Vec;
use bit_field::BitField;
use color::Color;
use conquer_once::spin::OnceCell;
use frame_tracker::FrameTracker;
use spin::mutex::Mutex;
use uefi::proto::console::gop::{ModeInfo, PixelFormat};

use common::boot::BootInformation;

use crate::{
    log,
    memory::{memory_controller, paging::EntryFlags, virt_addr_alloc},
};

pub mod color;
mod frame_tracker;

pub static DRIVER: OnceCell<Mutex<Graphic>> = OnceCell::uninit();

#[derive(Clone)]
struct GlyphData {
    start: usize,
    size: usize,
    width: usize,
    height: usize,
}

pub struct Graphic {
    mode: ModeInfo,
    plot_fn: for<'a> unsafe fn(&'a mut Self, color: Color, y: usize, x: usize),
    #[allow(unused)]
    get_pixel_fn: for<'a> fn(&'a Self, x: usize, y: usize) -> Color,
    frame_buffer: &'static mut [u32],
    real_buffer: &'static mut [u32],
    backbuffer_tracker: FrameTracker,
    glyph_tracker: FrameTracker,
    glyphs: Vec<u32>,
    glyph_ids: Vec<GlyphData>,
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
            backbuffer_tracker: FrameTracker::new(width, height, mode.stride()),
            glyph_tracker: FrameTracker::new(width, height, mode.stride()),
            glyphs: Vec::new(),
            glyph_ids: Vec::new(),
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
        let min_pos = self.backbuffer_tracker.frame_buffer_min();
        let max_pos = self
            .backbuffer_tracker
            .frame_buffer_max()
            .min(self.real_buffer.len() - 1);
        self.real_buffer[min_pos..=max_pos].copy_from_slice(&self.frame_buffer[min_pos..=max_pos]);
        self.backbuffer_tracker.reset();
    }

    /// Create a new glyph and returns it's id
    pub fn new_glyph<F>(&mut self, render: F) -> usize
    where
        F: FnOnce(&mut Self),
    {
        self.glyph_tracker.reset();
        render(self);
        let glyph_min = self.glyph_tracker.frame_buffer_min();
        let glyph_max = self.glyph_tracker.frame_buffer_max();
        let glyph = &self.frame_buffer[glyph_min..=glyph_max];
        self.glyph_ids.push(GlyphData {
            start: self.glyphs.len(),
            size: glyph.len(),
            width: self.glyph_tracker.frame_width(),
            height: self.glyph_tracker.frame_height(),
        });
        self.glyphs.extend_from_slice(glyph);
        self.glyph_ids.len() - 1
    }

    pub fn plot_glyph(&mut self, x: usize, y: usize, glyph_id: usize) {
        let (width, height) = self.mode.resolution();

        if x >= width || y >= height {
            return;
        }

        let glyph_data = match self.glyph_ids.get(glyph_id) {
            Some(glyph) => glyph,
            None => {
                log!(Error, "Invalid glyph id");
                return;
            }
        }
        .clone();
        let glyph = match self
            .glyphs
            .get(glyph_data.start..(glyph_data.start + glyph_data.size))
        {
            Some(glyph) => glyph,
            None => {
                log!(Error, "Invalid glyph data");
                return;
            }
        };
        let stride = self.mode.stride();
        let start_x = x;
        let start_y = y;

        for yy in 0..glyph_data.height {
            let fb_offset = (start_y + yy) * stride + start_x;
            let glyph_offset = yy * stride;

            if start_y + yy >= height {
                continue;
            }

            self.frame_buffer[fb_offset..fb_offset + glyph_data.width]
                .copy_from_slice(&glyph[glyph_offset..glyph_offset + glyph_data.width]);
        }
        self.backbuffer_tracker.track(x, y);
        self.backbuffer_tracker
            .track(x + glyph_data.width, y + glyph_data.height);
    }

    pub fn plot(&mut self, x: usize, y: usize, color: Color) {
        let (width, height) = self.mode.resolution();
        if x >= width || y >= height {
            return;
        }

        self.backbuffer_tracker.track(x, y);
        self.glyph_tracker.track(x, y);

        unsafe {
            (self.plot_fn)(self, color, y, x);
        }
    }

    pub fn scroll_up(&mut self, scroll_amount: usize) {
        let (_width, height) = self.mode.resolution();
        self.backbuffer_tracker.track_all();

        let stride = self.mode.stride();
        self.frame_buffer
            .copy_within(stride * scroll_amount..stride * height, 0);

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

pub fn graphic() -> &'static Mutex<Graphic> {
    DRIVER.get().expect("Uninitialize graphics")
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
