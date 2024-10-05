use core::usize;

use alloc::{string::String, vec::Vec};
use fontdue::{Font, FontSettings, Metrics};
use hashbrown::HashMap;

use crate::{
    graphics::{self, color::Color},
    memory::memory_controller,
    BootInformation,
};

const PIXEL_SIZE: usize = 25;
pub struct TtfRenderer {
    data: Vec<char>,
    cache: HashMap<char, (Metrics, Vec<u8>)>,
    foreground_color: Color,
    background_color: Color,
    font: Font,
    y_offset: usize,
    modified_fg_color: Option<Color>,
}

impl TtfRenderer {
    pub fn new(
        boot_info: &BootInformation,
        foreground_color: Color,
        background_color: Color,
    ) -> Self {
        memory_controller().lock().ident_map(
            boot_info.font_size() as u64,
            boot_info
                .font_addr()
                .expect("Failed to get font for mapping"),
        );
        let font = Font::from_bytes(
            boot_info.font().expect("Failed to get font ownership"),
            FontSettings::default(),
        )
        .unwrap();
        Self {
            data: Vec::with_capacity(5000),
            foreground_color,
            background_color,
            font,
            modified_fg_color: None,
            y_offset: 0,
            cache: HashMap::with_capacity(255),
        }
    }

    pub fn set_color(&mut self, color: Color) {
        self.foreground_color = color;
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

    pub fn update(&mut self) {
        let mut graphics = graphics::DRIVER.get().unwrap().lock();
        let mut offset = 1;
        let mut y_offset = 0;
        let (horizontal, vertical) = graphics.get_res();
        let max_lines = vertical / PIXEL_SIZE; // Calculate maximum lines
        let mut iter = self.data.iter().peekable();
        let mut next_y_offset = 0;
        while let Some(charactor) = iter.next() {
            if *charactor == '\n' {
                y_offset += 1;
                offset = 1;
                continue;
            }
            if *charactor == ' ' {
                offset += 8;
                if offset > horizontal - 10 {
                    y_offset += 1;
                    offset = 1;
                }
                continue;
            }
            if *charactor == '\x1b' {
                if iter.next_if(|e| **e == '[').is_some() {
                    let mut buf = String::new();
                    while let Some(number) = iter.next() {
                        if number.is_numeric() {
                            buf.push(*number);
                        } else {
                            break;
                        }
                    }
                    if let Ok(yipee) = buf.parse::<u8>() {
                        self.modified_fg_color = match yipee {
                            30 => Some(Color::new(0, 0, 0)),       // BLACK,
                            31 => Some(Color::new(170, 0, 0)),     // RED
                            32 => Some(Color::new(0, 170, 0)),     // GREEN
                            33 => Some(Color::new(255, 199, 6)),   // YELLOW
                            34 => Some(Color::new(0, 0, 170)),     // BLUE
                            35 => Some(Color::new(118, 38, 113)),  // MAGENTA
                            36 => Some(Color::new(0, 170, 170)),   // CYAN
                            37 => Some(Color::new(192, 192, 192)), // WHITE
                            90 => Some(Color::new(85, 85, 85)),    // BRIGHT BLACK
                            91 => Some(Color::new(255, 0, 0)),     // BRIGHT RED
                            92 => Some(Color::new(85, 255, 85)),   // BRIGHT GREEN
                            93 => Some(Color::new(255, 255, 0)),   // BRIGHT YELLOW
                            94 => Some(Color::new(0, 0, 255)),     // BRIGHT BLUE
                            95 => Some(Color::new(255, 0, 255)),   // BRIGHT MAGENTA
                            96 => Some(Color::new(0, 255, 255)),   // BRIGHT CYAN
                            97 => Some(Color::new(255, 255, 255)), // BRIGHT WHITE
                            0 => None,
                            _ => self.modified_fg_color,
                        }
                    }
                    iter.next_if(|e| **e == 'm');
                }
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

            if y_offset as i32 - self.y_offset as i32 / PIXEL_SIZE as i32 >= max_lines as i32 {
                graphics.scroll_up(PIXEL_SIZE);
                next_y_offset += PIXEL_SIZE;
                y_offset = max_lines - 1;
            }

            for pixel in bitmap.iter().rev() {
                let color = match self.modified_fg_color {
                    Some(modcolor) => modcolor,
                    None => self.foreground_color,
                };
                graphics.plot(
                    x + offset,
                    ((y + y_offset * PIXEL_SIZE) as i32 - metrics.ymin - self.y_offset as i32)
                        .try_into()
                        .unwrap_or(horizontal),
                    color.blend(self.background_color, *pixel as f32 / 255.0),
                );
                x -= 1;
                if x <= 0 {
                    y -= 1;
                    x = metrics.width;
                }
            }
            offset += metrics.width as usize + 2;
            if offset > horizontal - 10 {
                y_offset += 1;
                offset = 1;
            }
        }
        self.y_offset += next_y_offset;
    }
}
