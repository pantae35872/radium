// TODO: change this to u32 for optimization
#[derive(Clone, Copy, Debug)]
pub struct Color {
    r: u8,
    g: u8,
    b: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn blend(self, other: Self, alpha: f32) -> Self {
        Self {
            r: (self.r as f32 * alpha + other.r as f32 * (1.0 - alpha)) as u8,
            g: (self.g as f32 * alpha + other.g as f32 * (1.0 - alpha)) as u8,
            b: (self.b as f32 * alpha + other.b as f32 * (1.0 - alpha)) as u8,
        }
    }

    pub fn as_u32(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    pub fn apply_bitmask(self, r: u32, g: u32, b: u32) -> u32 {
        let red = ((self.r as u32) << r.trailing_zeros()) & self.r as u32;
        let green = ((self.g as u32) << g.trailing_zeros()) & self.g as u32;
        let blue = ((self.b as u32) << b.trailing_zeros()) & self.b as u32;

        return red | green | blue;
    }

    pub fn increase_brightness(&mut self, factor: f32) {
        if factor < 0.0 {
            return;
        }

        self.r = ((self.r as f32 * factor).min(255.0)) as u8;
        self.g = ((self.g as f32 * factor).min(255.0)) as u8;
        self.b = ((self.b as f32 * factor).min(255.0)) as u8;
    }
}
