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
