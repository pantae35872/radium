use alloc::vec::Vec;
use bit_field::BitField;
use bootbridge::{BootBridge, GraphicsInfo, PixelFormat};
use color::Color;
use conquer_once::spin::OnceCell;
use core::arch::asm;
use frame_tracker::FrameTracker;
use pager::{address::Page, EntryFlags, Mapper, PAGE_SIZE};
use spin::mutex::Mutex;

use crate::{
    log,
    memory::{virt_addr_alloc, MMIOBuffer, MMIOBufferInfo, MMIODevice, MemoryContext},
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
    mode: GraphicsInfo,
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
    /// Performs a backbuffer swap
    pub fn swap(&mut self) {
        let min_pos = self.backbuffer_tracker.frame_buffer_min();
        let max_pos = self
            .backbuffer_tracker
            .frame_buffer_max()
            .min(self.real_buffer.len() - 1);
        unsafe {
            let src = &self.frame_buffer[min_pos..=max_pos];
            let dst = &mut self.real_buffer[min_pos..=max_pos];
            let dst_ptr = &mut dst[0] as *mut u32 as *mut u8;
            let src_ptr = &src[0] as *const u32 as *const u8;
            let src_len = src.len() * 4;
            Self::memmove_sse(dst_ptr, src_ptr, src_len);
        }
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

            unsafe {
                let dest = self.frame_buffer.as_mut_ptr().add(fb_offset);
                let src = glyph.as_ptr().add(glyph_offset);
                Self::memmove_sse(dest as *mut u8, src as *const u8, glyph_data.width * 4);
            }
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

    /// This function is only intended to be used by this module
    unsafe fn memmove_sse(mut dest: *mut u8, mut src: *const u8, count: usize) {
        let mut i = 0;

        while count - i >= 128 {
            unsafe {
                asm!(
                    "movdqu xmm0, [{src}]",
                    "movdqu xmm1, [{src} + 16]",
                    "movdqu xmm2, [{src} + 32]",
                    "movdqu xmm3, [{src} + 48]",
                    "movdqu xmm4, [{src} + 64]",
                    "movdqu xmm5, [{src} + 80]",
                    "movdqu xmm6, [{src} + 96]",
                    "movdqu xmm7, [{src} + 112]",
                    "movdqu [{dst}], xmm0",
                    "movdqu [{dst} + 16], xmm1",
                    "movdqu [{dst} + 32], xmm2",
                    "movdqu [{dst} + 48], xmm3",
                    "movdqu [{dst} + 64], xmm4",
                    "movdqu [{dst} + 80], xmm5",
                    "movdqu [{dst} + 96], xmm6",
                    "movdqu [{dst} + 112], xmm7",
                    src = in(reg) src,
                    dst = in(reg) dest,
                    options(nostack, preserves_flags),
                );
                src = src.add(128);
                dest = dest.add(128);
            }
            i += 128;
        }

        let remaining = count - i;
        if remaining > 0 {
            unsafe {
                core::ptr::copy(src.add(i), dest.add(i), remaining);
            }
        }
    }

    pub fn scroll_up(&mut self, scroll_amount: usize) {
        let (_width, height) = self.mode.resolution();
        self.backbuffer_tracker.track_all();

        let stride = self.mode.stride();

        unsafe {
            let src = &self.frame_buffer[stride * scroll_amount..stride * height];
            let src_ptr = &src[0] as *const u32 as *const u8;
            let src_len = src.len() * 4;
            Self::memmove_sse(
                &mut self.frame_buffer[0] as *mut u32 as *mut u8,
                src_ptr,
                src_len,
            );
        }

        self.frame_buffer[(self.mode.stride() * (height - scroll_amount))..].fill(
            match self.mode.pixel_format() {
                PixelFormat::Rgb => BACKGROUND_COLOR.as_u32() << 8,
                PixelFormat::Bgr => BACKGROUND_COLOR.as_u32(),
                PixelFormat::Bitmask(bitmask) => {
                    BACKGROUND_COLOR.apply_bitmask(bitmask.red, bitmask.green, bitmask.blue)
                }
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
        match self.mode.pixel_format() {
            PixelFormat::Bitmask(bitmask) => {
                self.frame_buffer[y * self.mode.stride() + x] =
                    color.apply_bitmask(bitmask.red, bitmask.green, bitmask.blue);
            }
            _ => {}
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
        match self.mode.pixel_format() {
            PixelFormat::Bitmask(bitmask) => {
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
            _ => Color::new(0, 0, 0),
        }
    }

    pub fn get_res(&self) -> (usize, usize) {
        return self.mode.resolution();
    }
}

pub fn graphic() -> &'static Mutex<Graphic> {
    DRIVER.get().expect("Uninitialize graphics")
}

impl MMIODevice<(&'static mut [u32], GraphicsInfo)> for Graphic {
    fn boot_bridge(bootbridge: &BootBridge) -> Option<MMIOBufferInfo> {
        Some(bootbridge.framebuffer_data().into())
    }

    fn new(buffer: MMIOBuffer, args: (&'static mut [u32], GraphicsInfo)) -> Self {
        let (back_buffer, mode) = args;
        let (width, height) = mode.resolution();
        log!(Info, "Graphic resolution {}x{}", width, height);
        let plot_fn = match mode.pixel_format() {
            PixelFormat::Rgb => Self::plot_rgb,
            PixelFormat::Bgr => Self::plot_bgr,
            PixelFormat::Bitmask(_) => Self::plot_bitmask,
            PixelFormat::BltOnly => unimplemented!("Not support"),
        };
        let get_pixel_fn = match mode.pixel_format() {
            PixelFormat::Rgb => Self::get_pixel_rgb,
            PixelFormat::Bgr => Self::get_pixel_bgr,
            PixelFormat::Bitmask(_) => Self::get_pixel_bitmask,
            PixelFormat::BltOnly => unimplemented!("Not support"),
        };
        let mut va = Self {
            mode,
            plot_fn,
            get_pixel_fn,
            real_buffer: buffer.as_slice(),
            frame_buffer: back_buffer,
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
}

pub fn init(ctx: &mut MemoryContext, boot_bridge: &BootBridge) {
    log!(Trace, "Registering graphic");
    let graphics_info = boot_bridge.graphics_info();
    let start = virt_addr_alloc(boot_bridge.framebuffer_data().size() as u64 / PAGE_SIZE);
    ctx.mapper().map_range(
        start,
        Page::containing_address(start.start_address() + boot_bridge.framebuffer_data().size() - 1),
        EntryFlags::WRITABLE,
    );
    let graphics = ctx
        .mmio_device::<Graphic, _>(
            (
                unsafe {
                    core::slice::from_raw_parts_mut(
                        start.start_address().as_mut_ptr::<u32>(),
                        boot_bridge.framebuffer_data().size() / size_of::<u32>(),
                    )
                },
                graphics_info,
            ),
            boot_bridge,
            None,
        )
        .expect("Failed to create graphics driver");
    DRIVER.init_once(|| graphics.into());
}
