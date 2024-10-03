use core::usize;

use alloc::vec::Vec;
use fontdue::{Font, FontSettings, Metrics};
use hashbrown::HashMap;

use crate::{graphics, BootInformation};

const PIXEL_SIZE: usize = 18;
pub struct TtfRenderer {
    data: Vec<char>,
    cache: HashMap<char, (Metrics, Vec<u8>)>,
    foreground_color: u32,
    font: Font,
}

impl TtfRenderer {
    pub fn new(boot_info: &BootInformation, foreground_color: u32) -> Self {
        let font = Font::from_bytes(
            boot_info.font().expect("Failed to get font ownership"),
            FontSettings::default(),
        )
        .unwrap();
        Self {
            data: Vec::with_capacity(5000),
            foreground_color,
            font,
            cache: HashMap::with_capacity(255),
        }
    }

    pub fn set_color(&mut self, color: &u32) {
        self.foreground_color = *color;
    }

    pub fn put_char(&mut self, charactor: &char) {
        self.data.push(*charactor);
    }

    pub fn put_str(&mut self, string: &str) {
        for char in string.chars() {
            self.put_char(&char);
        }
        self.update();
    }

    pub fn cache(&mut self, charactor: &char) -> bool {
        match self.cache.get_mut(charactor) {
            Some(_) => {
                return true;
            }
            None => {
                let font = self.font.rasterize(*charactor, PIXEL_SIZE as f32);
                self.cache.insert(*charactor, font);
                return false;
            }
        };
    }

    fn adjust_brightness(color: u32, alpha: u8) -> u32 {
        let alpha = alpha as f32 / 255.0;

        let red = (color >> 16) & 0xFF;
        let green = (color >> 8) & 0xFF;
        let blue = color & 0xFF;
        let new_red = (red as f32 * alpha) as u32;
        let new_green = (green as f32 * alpha) as u32;
        let new_blue = (blue as f32 * alpha) as u32;

        (new_red << 16) | (new_green << 8) | new_blue
    }

    pub fn update(&mut self) {
        let mut graphics = graphics::DRIVER.get().unwrap().lock();
        let mut offset = 1;
        let mut y_offset = 0;
        let (horizontal, _vertical) = graphics.get_res();
        for charactor in &self.data {
            if *charactor == '\n' {
                y_offset += 1;
                offset = 1;
                continue;
            }
            let (metrics, bitmap) = match self.cache.get_mut(charactor) {
                Some(polygon) => polygon,
                None => {
                    let font = self.font.rasterize(*charactor, PIXEL_SIZE as f32);
                    self.cache.insert(*charactor, font);
                    self.cache.get_mut(charactor).unwrap()
                }
            };
            let mut x = metrics.width;
            let mut y = PIXEL_SIZE;
            for pixel in bitmap.iter().rev() {
                graphics.plot(
                    x + offset,
                    ((y + y_offset * PIXEL_SIZE) as i32 - metrics.ymin) as usize,
                    Self::adjust_brightness(self.foreground_color, *pixel),
                );
                x -= 1;
                if x <= 0 {
                    y -= 1;
                    x = metrics.width;
                }
            }
            offset += metrics.advance_width as usize + 1;
            if offset > horizontal - 10 {
                y_offset += 1;
                offset = 1;
            }
        }
    }
}
