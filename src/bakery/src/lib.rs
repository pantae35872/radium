#![no_std]

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
pub use std::{string::String, vec::Vec};

#[cfg(all(not(feature = "std"), feature = "alloc"))]
pub use alloc::{string::String, vec::Vec};

use pager::{DataBuffer, IdentityMappable};

const MAGIC: u32 = u32::from_le_bytes(*b"BAKE");

#[repr(C)]
struct BakeryString {
    // Offset in the string table
    offset: u64,
    size: u64,
}

#[repr(C)]
struct BakerySymbol {
    addr: u64,
    end: u64,
    line_num: u32,
    name: BakeryString,
    location: BakeryString,
}

#[derive(Debug)]
pub struct DwarfBaker<'a> {
    data: DataBuffer<'a>,
}

pub struct Bakery {
    file_buffer: Vec<u8>,
    string_buffer: String,
}

impl Bakery {
    pub fn new() -> Self {
        Self {
            file_buffer: Vec::new(),
            string_buffer: String::new(),
        }
    }

    pub fn push(&mut self, addr: u64, end: u64, line_num: u32, name: &str, location: &str) {
        let location_name = self.string_buffer.len();
        self.string_buffer += name;
        let location_offset = self.string_buffer.len();
        self.string_buffer += location;
        self.file_buffer.extend_from_slice(&unsafe {
            core::mem::transmute::<BakerySymbol, [u8; size_of::<BakerySymbol>()]>(BakerySymbol {
                addr,
                end,
                line_num,
                name: BakeryString {
                    offset: location_name as u64,
                    size: name.len() as u64,
                },
                location: BakeryString {
                    offset: location_offset as u64,
                    size: location.len() as u64,
                },
            })
        });
    }

    pub fn bake(&self) -> Vec<u8> {
        let mut result = Vec::new();

        result.extend_from_slice(&MAGIC.to_le_bytes());
        result.extend_from_slice(&self.file_buffer.len().to_le_bytes());
        result.extend_from_slice(&self.file_buffer);
        result.extend_from_slice(&self.string_buffer.len().to_le_bytes());
        result.extend_from_slice(self.string_buffer.as_bytes());

        result
    }
}

impl Default for Bakery {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> DwarfBaker<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        assert!(data[0..4] == MAGIC.to_le_bytes());
        Self {
            data: DataBuffer::new(data),
        }
    }

    fn symbol(&self, index: usize) -> Option<BakerySymbol> {
        let mut data = self.data.buffer();
        data = &data[size_of::<u32>()..];
        let file_len =
            usize::from_le_bytes(TryInto::try_into(&data[..size_of::<usize>()]).unwrap());
        data = &data[size_of::<usize>()..];
        let start = index * size_of::<BakerySymbol>();
        if start + size_of::<BakerySymbol>() > file_len {
            return None;
        }
        Some(unsafe {
            core::mem::transmute::<[u8; size_of::<BakerySymbol>()], BakerySymbol>(
                data[start..start + size_of::<BakerySymbol>()]
                    .try_into()
                    .unwrap(),
            )
        })
    }

    fn symbol_len(&self) -> usize {
        let mut data = self.data.buffer();
        data = &data[size_of::<u32>()..];
        let file_len =
            usize::from_le_bytes(TryInto::try_into(&data[..size_of::<usize>()]).unwrap());
        file_len / size_of::<BakerySymbol>()
    }

    fn string_table(&self) -> &'a str {
        let mut data = self.data.buffer();
        data = &data[size_of::<u32>()..];
        let file_len =
            usize::from_le_bytes(TryInto::try_into(&data[..size_of::<usize>()]).unwrap());
        data = &data[size_of::<usize>()..];
        data = &data[file_len..];
        let table_len =
            usize::from_le_bytes(TryInto::try_into(&data[..size_of::<usize>()]).unwrap());
        data = &data[size_of::<usize>()..];
        data = &data[..table_len];
        str::from_utf8(data).expect("Invalid utf8 in dwarf")
    }

    fn string(&self, string: &BakeryString) -> &'a str {
        &self.string_table()[string.offset as usize..string.offset as usize + string.size as usize]
    }

    pub fn by_addr(&self, addr: u64) -> Option<(u32, &'a str, &'a str)> {
        let mut left = 0;
        let mut right = self.symbol_len();

        while left < right {
            let mid = (left + right) / 2;
            let entry = &self.symbol(mid)?;

            if addr < entry.addr {
                right = mid;
            } else if addr >= entry.end {
                left = mid + 1;
            } else {
                return Some((
                    entry.line_num,
                    self.string(&entry.name),
                    self.string(&entry.location),
                ));
            }
        }

        None
    }
}

impl IdentityMappable for DwarfBaker<'_> {
    fn map(&self, mapper: &mut impl pager::Mapper) {
        self.data.map(mapper);
    }
}
