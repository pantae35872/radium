use core::{isize, usize};

use alloc::{string::String, vec::Vec};
use fontdue::{Font, FontSettings, Metrics};
use hashbrown::HashMap;

use crate::{
    graphics::{color::Color, graphic},
    memory::{memory_controller, paging::EntryFlags},
    BootInformation,
};

#[derive(Hash, PartialEq, Eq)]
struct Glyph {
    color: u32,
    character: char,
}

pub struct TtfRenderer {
    data: Vec<char>,
    cache: HashMap<char, (Metrics, Vec<u8>)>,
    foreground_color: Color,
    background_color: Color,
    font: Font,
    initial_offset: isize,
    modified_fg_color: Option<Color>,
    current_line: u64,
    glyph_cache: HashMap<Glyph, usize>,
    pixel_size: usize,
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
            EntryFlags::WRITABLE | EntryFlags::PRESENT | EntryFlags::NO_CACHE,
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
            current_line: 0,
            font,
            modified_fg_color: None,
            initial_offset: 0,
            cache: HashMap::with_capacity(255),
            glyph_cache: HashMap::with_capacity(255),
            pixel_size: boot_info.font_pixel_size(),
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

    pub fn update(&mut self) {
        let mut graphics = graphic().lock();
        let mut offset = 1;
        let mut y_offset = self.initial_offset;
        let (horizontal, vertical) = graphics.get_res();
        let max_lines = vertical / self.pixel_size - 1; // Calculate maximum lines
        let mut iter = self.data.iter().peekable();
        let mut current_line = 0;
        while let Some(character) = iter.next() {
            if *character == '\n' {
                y_offset += 1;
                current_line += 1;
                offset = 1;
                continue;
            }
            if *character == ' ' {
                offset += 8;
                if offset > horizontal - 10 {
                    y_offset += 1;
                    current_line += 1;
                    offset = 1;
                }
                continue;
            }
            if *character == '\x1b' {
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
            let (metrics, bitmap) = match self.cache.get_mut(character) {
                Some(polygon) => polygon,
                None => {
                    let font = self.font.rasterize(*character, self.pixel_size as f32);
                    self.cache.insert(*character, font);
                    self.cache.get_mut(character).unwrap()
                }
            };

            if offset + metrics.width > horizontal {
                y_offset += 1;
                current_line += 1;
                offset = 1;
            }

            let mut x = metrics.width;
            let mut y = self.pixel_size;

            if y_offset as i32 >= max_lines as i32 {
                graphics.scroll_up(self.pixel_size);
                y_offset = max_lines as isize - 1;
                self.initial_offset -= 1;
            }

            if y_offset >= 0 && current_line >= self.current_line {
                let color = match self.modified_fg_color {
                    Some(modcolor) => modcolor,
                    None => self.foreground_color,
                };
                self.glyph_cache
                    .entry(Glyph {
                        character: *character,
                        color: color.as_u32(),
                    })
                    .and_modify(|id| {
                        graphics.plot_glyph(
                            offset + 1,
                            (y_offset * self.pixel_size as isize
                                + (y as isize - metrics.height as isize)
                                - metrics.ymin as isize
                                + 1)
                            .try_into()
                            .unwrap_or(horizontal),
                            *id,
                        );
                    })
                    .or_insert_with(|| {
                        graphics.new_glyph(|graphics| {
                            for pixel in bitmap.iter().rev() {
                                graphics.plot(
                                    x + offset,
                                    ((y as isize + y_offset * self.pixel_size as isize)
                                        - metrics.ymin as isize)
                                        .try_into()
                                        .unwrap_or(horizontal),
                                    color.blend(self.background_color, *pixel as f32 / 255.0),
                                );
                                x -= 1;
                                if x <= 0 {
                                    y = y.saturating_sub(1);
                                    x = metrics.width;
                                }
                            }
                        })
                    });
            }

            offset += metrics.width as usize + 2;
        }
        if self.current_line != current_line {
            graphics.swap();
        }
        self.current_line = self.current_line.max(current_line);
    }
}
