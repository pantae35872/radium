use std::iter;

#[derive(Debug, Clone, Default)]
pub struct Writer {
    buffer: Vec<u8>,
}

macro_rules! impl_write_type {
    ($ty: ty) => {
        paste::paste! {
            pub fn [< write_$ty >](&mut self, value: $ty) -> &mut Self {
                self.write_bytes(&value.to_le_bytes());
                self
            }
        }
    };
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> &mut Self {
        self.buffer.extend_from_slice(bytes);
        self
    }

    pub fn write_u8(&mut self, value: u8) -> &mut Self {
        self.write_bytes(&value.to_le_bytes());
        self
    }

    pub fn write_i8(&mut self, value: i8) -> &mut Self {
        self.write_bytes(&value.to_le_bytes());
        self
    }

    pub fn write_str_padded_with(&mut self, value: impl AsRef<str>, length: usize, pad_with: u8) -> &mut Self {
        let str_bytes = value.as_ref().as_bytes();
        let before_len = self.len();
        if str_bytes.len() > length {
            self.write_bytes(&str_bytes[0..length]);
        } else {
            self.write_bytes(&str_bytes);
            for _ in str_bytes.len()..length {
                self.buffer.push(pad_with);
            }
        }
        assert_eq!(before_len + length, self.len());
        self
    }

    pub fn write_str_padded(&mut self, value: impl AsRef<str>, length: usize) -> &mut Self {
        self.write_str_padded_with(value, length, 0x20)
    }

    pub fn write_str(&mut self, value: impl AsRef<str>) -> &mut Self {
        self.write_bytes(value.as_ref().as_bytes());
        self
    }

    /// Pad the buffer to the minimum size
    pub fn padded_min_with(&mut self, min: usize, value: u8) {
        self.buffer.extend(iter::repeat_n(value, min - self.len()));
    }

    /// Pad the buffer to the minimum size
    pub fn padded_min(&mut self, min: usize) {
        self.buffer.extend(iter::repeat_n(0u8, min - self.len()));
    }

    /// Pad the buffer
    pub fn padded(&mut self, amount: usize) {
        self.buffer.extend(iter::repeat_n(0u8, amount));
    }

    impl_write_type!(u16);
    impl_write_type!(i16);
    impl_write_type!(u32);
    impl_write_type!(i32);

    pub fn buffer(&self) -> &[u8] {
        &self.buffer
    }

    pub fn buffer_owned(self) -> Vec<u8> {
        self.buffer
    }
}
