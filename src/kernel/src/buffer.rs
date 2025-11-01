use bincode::{
    de::read::{BorrowReader, Reader},
    enc::write::Writer,
    error::DecodeError,
};
use thiserror::Error;

use crate::buffer::{read::BufferReader, write::BufferWriter};

pub mod read;
pub mod write;

#[derive(Debug, Error)]
pub enum BufferReadError {
    #[error("Out of range")]
    OutOfRange,
}

pub enum Endian {
    Big,
    Little,
}

impl<'a> Writer for BufferWriter<'a> {
    fn write(&mut self, bytes: &[u8]) -> Result<(), bincode::error::EncodeError> {
        self.write_bytes(bytes);
        Ok(())
    }
}

impl<'a> Reader for BufferReader<'a> {
    fn read(&mut self, bytes: &mut [u8]) -> Result<(), bincode::error::DecodeError> {
        let read_bytes = self
            .read_bytes(bytes.len())
            .map_err(|_e| DecodeError::UnexpectedEnd {
                additional: bytes.len() - self.len(),
            })?;
        bytes.copy_from_slice(read_bytes);
        Ok(())
    }
}

impl<'storage> BorrowReader<'storage> for BufferReader<'storage> {
    fn take_bytes(&mut self, length: usize) -> Result<&'storage [u8], DecodeError> {
        let read_bytes = self
            .read_bytes(length)
            .map_err(|_e| DecodeError::UnexpectedEnd {
                additional: length - self.len(),
            })?;
        Ok(read_bytes)
    }
}

#[cfg(test)]
mod test {
    use alloc::vec::Vec;

    use crate::buffer::{read::BufferReader, write::BufferWriter};

    #[derive(Debug, Default)]
    struct WriterReaderExpect {
        wrote: Vec<WriteValue>,
    }

    macro_rules! gen_check {
        (enum $name: ident {
            $($field: ident($types: ty) -> ($read_fn: ident, $write_fn: ident)),* $(,)?
        }) => {
            #[derive(Debug)]
            enum $name {
                $($field($types)),*
            }

            impl WriterReaderExpect {
                pub fn new() -> Self {
                    Self::default()
                }

                pub fn write(&mut self, writer: &mut BufferWriter, value: WriteValue) {
                    match value {
                        $($name::$field(value) => writer.$write_fn(value),)*
                    }
                    self.wrote.push(value);
                }

                pub fn check(&self, reader: &mut BufferReader) {
                    for wrote in self.wrote.iter() {
                        match wrote {
                            $($name::$field(value) => assert_eq!(reader.$read_fn().expect("value"), *value),)*
                        }
                    }
                }
            }
        };
    }

    gen_check! {
        enum WriteValue {
            U8 (u8)  -> (read_byte, write_byte),
            U16(u16) -> (read_u16, write_u16),
            U32(u32) -> (read_u32, write_u32),
            U64(u64) -> (read_u64, write_u64),
            I8 (i8)  -> (read_i8, write_i8),
            I16(i16) -> (read_i16, write_i16),
            I32(i32) -> (read_i32, write_i32),
            I64(i64) -> (read_i64, write_i64),
        }
    }

    macro_rules! test_write {
        ([$($name: ident($value: expr)),* $(,)?] -> $buffer: ident) => {{
            let mut writer = BufferWriter::new(&mut $buffer);
            let mut expect = WriterReaderExpect::new();
            $(expect.write(&mut writer, WriteValue::$name($value));)*
            expect
        }};
    }

    #[test_case]
    fn writer_reader() {
        let mut buffer = Vec::new();
        let expect = test_write!([U16(16), U32(122), U64(444444), I8(11), U8(244)] -> buffer);
        let mut reader = BufferReader::new(&buffer);
        expect.check(&mut reader);
    }
}
