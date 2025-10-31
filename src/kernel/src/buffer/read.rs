use super::Endian;
use crate::buffer::BufferReadError;

pub struct BufferReader<'a> {
    buffer: &'a [u8],
    index: usize,
}

macro_rules! impl_read {
    ($name:ident, $ty:ty, $len:expr) => {
        paste::paste! {
            pub fn $name(&mut self) -> Result<$ty, BufferReadError> {
                self.[< $name _with_endian>](Endian::Little)
            }
            pub fn [< $name _with_endian >](&mut self, endian: Endian) -> Result<$ty, BufferReadError> {
                let bytes = self.read_bytes($len)?;
                let arr: [u8; $len] = bytes.try_into().expect(concat!(
                    "The length of read bytes is somehow not ",
                    stringify!($len)
                ));
                Ok(match endian {
                    Endian::Big => <$ty>::from_be_bytes(arr),
                    Endian::Little => <$ty>::from_le_bytes(arr),
                })
            }
        }
    };
}

impl<'a> BufferReader<'a> {
    pub fn new(buffer: &'a [u8]) -> Self {
        Self { buffer, index: 0 }
    }

    pub fn read_byte(&mut self) -> Result<u8, BufferReadError> {
        if self.index >= self.buffer.len() {
            return Err(BufferReadError::OutOfRange);
        }
        let value = self.buffer[self.index];
        self.index += 1;
        Ok(value)
    }

    pub fn get_index(&self) -> usize {
        self.index
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn read_bytes(&mut self, amount: usize) -> Result<&'a [u8], BufferReadError> {
        if self.index + amount > self.buffer.len() {
            return Err(BufferReadError::OutOfRange);
        }
        let result = &self.buffer[self.index..(self.index + amount)];
        self.index += amount;
        Ok(result)
    }

    impl_read!(read_i8, i8, 1);
    impl_read!(read_i16, i16, 2);
    impl_read!(read_i32, i32, 4);
    impl_read!(read_i64, i64, 8);

    impl_read!(read_u16, u16, 2);
    impl_read!(read_u32, u32, 4);
    impl_read!(read_u64, u64, 8);

    pub fn skip(&mut self, amount: usize) -> Result<(), BufferReadError> {
        if self.index + amount > self.buffer.len() {
            return Err(BufferReadError::OutOfRange);
        }
        self.index += amount;
        Ok(())
    }
}
