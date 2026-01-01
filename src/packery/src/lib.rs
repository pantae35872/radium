#![no_std]

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
pub use std::{string::String, vec::Vec};

#[cfg(all(not(feature = "std"), feature = "alloc"))]
pub use alloc::{string::String, vec::Vec};

use pager::{DataBuffer, IdentityMappable, IdentityReplaceable};
use thiserror::Error;

const MAGIC: u32 = u32::from_le_bytes(*b"PACK");

pub struct Packery {
    entry_buffer: Vec<u8>,
    entry_length: u64,
    string_buffer: String,
    data_buffer: Vec<u8>,
}

impl Packery {
    pub fn new() -> Self {
        Self { entry_buffer: Vec::new(), entry_length: 0, string_buffer: String::new(), data_buffer: Vec::new() }
    }

    pub fn push(&mut self, name: &str, data: &[u8]) {
        let name_offset = self.string_buffer.len();
        self.string_buffer += name;
        let data_offset = self.data_buffer.len();
        self.data_buffer.extend_from_slice(data);
        self.entry_buffer.extend_from_slice(&unsafe {
            core::mem::transmute::<PackeryEntry, [u8; size_of::<PackeryEntry>()]>(PackeryEntry {
                data: PackeryData { offset: data_offset as u64, size: data.len() as u64 },
                name: PackeryString { offset: name_offset as u64, size: name.len() as u64 },
            })
        });
        self.entry_length += 1;
    }

    pub fn pack(&self) -> Vec<u8> {
        let mut result = Vec::new();

        result.extend_from_slice(&[0; size_of::<PackeryHeader>()]);
        let entry_offset = result.len() as u64;
        result.extend_from_slice(&self.entry_buffer);
        let string_offset = result.len() as u64;
        result.extend_from_slice(self.string_buffer.as_bytes());
        let data_offset = result.len() as u64;
        result.extend_from_slice(&self.data_buffer);
        result[0..size_of::<PackeryHeader>()].copy_from_slice(&unsafe {
            core::mem::transmute::<PackeryHeader, [u8; size_of::<PackeryHeader>()]>(PackeryHeader {
                magic: MAGIC,
                entry_offset,
                entry_length: self.entry_length,
                string_offset,
                string_length: self.string_buffer.len() as u64,
                data_offset,
                data_length: self.data_buffer.len() as u64,
            })
        });

        result
    }
}

impl Default for Packery {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
#[repr(C)]
struct PackeryData {
    // Offset in the string table
    offset: u64,
    size: u64,
}

#[derive(Debug)]
#[repr(C)]
struct PackeryString {
    // Offset in the string table
    offset: u64,
    size: u64,
}

#[derive(Debug)]
#[repr(C)]
struct PackeryEntry {
    name: PackeryString,
    data: PackeryData,
}

#[repr(C)]
struct PackeryHeader {
    magic: u32,
    entry_offset: u64,
    entry_length: u64,
    string_offset: u64,
    string_length: u64,
    data_offset: u64,
    data_length: u64,
}

#[derive(Debug)]
pub struct Packed<'a> {
    buffer: DataBuffer<'a>,
}

#[derive(Debug)]
pub struct ProgramContainer<'a> {
    pub name: &'a str,
    pub data: &'a [u8],
}

#[derive(Debug, Error)]
pub enum PackedError {
    #[error("Invalid packed file magic")]
    InvalidMagic,
    #[error("Out of range driver; index is {index} but length is {length}")]
    DriverIndexOutOfRange { index: usize, length: usize },
}

impl<'a> Packed<'a> {
    pub fn new(buffer: &'a [u8]) -> Result<Self, PackedError> {
        if buffer[0..4] != MAGIC.to_le_bytes() {
            return Err(PackedError::InvalidMagic);
        }
        Ok(Self { buffer: DataBuffer::new(buffer) })
    }

    fn header(&self) -> PackeryHeader {
        unsafe {
            core::mem::transmute::<[u8; size_of::<PackeryHeader>()], PackeryHeader>(
                self.buffer[..size_of::<PackeryHeader>()].try_into().unwrap(),
            )
        }
    }

    pub fn data_table(&'a self) -> &'a [u8] {
        let header = self.header();
        &self.buffer[header.data_offset as usize..header.data_offset as usize + header.data_length as usize]
    }

    pub fn string_table(&'a self) -> &'a str {
        let header = self.header();
        str::from_utf8(
            &self.buffer[header.string_offset as usize..header.string_offset as usize + header.string_length as usize],
        )
        .expect("Invalid utf8 in program pack")
    }

    fn get_data(&'a self, data: PackeryData) -> &'a [u8] {
        &self.data_table()[data.offset as usize..(data.offset + data.size) as usize]
    }

    fn get_string(&'a self, string: PackeryString) -> &'a str {
        &self.string_table()[string.offset as usize..(string.offset + string.size) as usize]
    }

    pub fn iter(&'a self) -> ProgramIter<'a> {
        ProgramIter { packed: self, index: 0 }
    }

    pub fn get_program(&'a self, index: usize) -> Result<ProgramContainer<'a>, PackedError> {
        let header = self.header();
        if index >= header.entry_length as usize {
            return Err(PackedError::DriverIndexOutOfRange { index, length: header.entry_length as usize });
        }

        let offset = header.entry_offset as usize + index * size_of::<PackeryEntry>();

        let entry = unsafe {
            core::mem::transmute::<[u8; size_of::<PackeryEntry>()], PackeryEntry>(
                self.buffer[offset..offset + size_of::<PackeryEntry>()].try_into().unwrap(),
            )
        };
        Ok(ProgramContainer { name: self.get_string(entry.name), data: self.get_data(entry.data) })
    }
}

#[derive(Debug)]
pub struct ProgramIter<'a> {
    packed: &'a Packed<'a>,
    index: usize,
}

impl<'a> Iterator for ProgramIter<'a> {
    type Item = ProgramContainer<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let before_index = self.index;
        self.index += 1;
        self.packed.get_program(before_index).ok()
    }
}

unsafe impl IdentityMappable for Packed<'_> {
    fn map(&self, mapper: &mut impl pager::Mapper) {
        self.buffer.map(mapper);
    }
}

unsafe impl IdentityReplaceable for Packed<'_> {
    fn identity_replace<T: pager::Mapper>(&mut self, mapper: &mut pager::MapperWithVirtualAllocator<T>) {
        self.buffer.identity_replace(mapper);
    }
}
