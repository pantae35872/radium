use super::Endian;
use alloc::vec::Vec;

pub struct BufferWriter<'a> {
    buffer: &'a mut Vec<u8>,
}

macro_rules! impl_write {
    ($name:ident, $ty:ty, $len:expr) => {
        paste::paste! {
            pub fn $name(&mut self, value: $ty) {
                self.[< $name _with_endian>](value, Endian::Little)
            }
            pub fn [< $name _with_endian >](&mut self, value: $ty, endian: Endian) {
                match endian {
                    Endian::Big => self.write_bytes(&value.to_be_bytes()),
                    Endian::Little => self.write_bytes(&value.to_le_bytes()),
                }
            }
        }
    };
}

impl<'a> BufferWriter<'a> {
    pub fn new(buffer: &'a mut Vec<u8>) -> Self {
        Self { buffer }
    }

    pub fn write_byte(&mut self, byte: u8) {
        self.buffer.push(byte);
    }

    pub fn write_bytes(&mut self, value: &[u8]) {
        self.buffer.extend_from_slice(value);
    }

    impl_write!(write_i8, i8, 1);
    impl_write!(write_i16, i16, 2);
    impl_write!(write_i32, i32, 4);
    impl_write!(write_i64, i64, 8);

    impl_write!(write_u16, u16, 2);
    impl_write!(write_u32, u32, 4);
    impl_write!(write_u64, u64, 8);
}
