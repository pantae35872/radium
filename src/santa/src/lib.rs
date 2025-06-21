#![no_std]
#![feature(custom_test_frameworks)]

extern crate alloc;

use core::fmt::Debug;
use core::iter::Iterator;
use pager::{
    address::{Frame, Page, PhysAddr, VirtAddr}, EntryFlags, IdentityMappable, VirtuallyMappable, VirtuallyReplaceable, PAGE_SIZE,
};
use reader::{ElfBits, ElfHeader, ElfReader, ProgramType};

mod reader;

#[derive(Debug)]
pub enum ElfError {
    InvalidHeader,
    Elf32BitNotSupport,
    /// The index into the string table is not valid from the ElfHeader
    InvalidStringTableIndex(usize),
    /// The string table in the elf file is not valid
    InvalidStringTable,
    InvalidMagic {
        magic: [u8; 4],
    },
}

// TODO: Add testing
#[derive(Debug)]
pub struct Elf<'a> {
    reader: ElfReader<'a>,
    mem_min: VirtAddr,
    mem_max: VirtAddr,
    max_memory_needed: usize,
    max_alignment: usize,
}

impl<'a> Elf<'a> {
    pub fn new(buffer: &'a [u8]) -> Result<Self, ElfError> {
        if buffer.len() < size_of::<ElfHeader>() {
            return Err(ElfError::InvalidHeader);
        }
        if buffer[0..4] != [0x7F, b'E', b'L', b'F'] {
            return Err(ElfError::InvalidMagic {
                magic: buffer[0..4].try_into().expect("Should not failed"),
            });
        }
        let reader = ElfReader::new(buffer);

        if reader.header().bits != ElfBits::B64 {
            return Err(ElfError::Elf32BitNotSupport);
        }

        let mut max_alignment: u64 = 4096;
        let mut mem_min: u64 = u64::MAX;
        let mut mem_max: u64 = 0;

        for header in reader.program_header_iter() {
            if header.segment_type() != ProgramType::Load {
                continue;
            }

            if max_alignment < header.alignment() {
                max_alignment = header.alignment();
            }

            let mut header_begin = header.vaddr().as_u64();
            let mut header_end = header.vaddr().as_u64() + header.memsize() + max_alignment - 1;

            header_begin &= !(max_alignment - 1);
            header_end &= !(max_alignment - 1);

            if header_begin < mem_min {
                mem_min = header_begin;
            }
            if header_end > mem_max {
                mem_max = header_end;
            }
        }

        Ok(Self {
            reader,
            mem_min: VirtAddr::new(mem_min),
            mem_max: VirtAddr::new(mem_max),
            max_memory_needed: (mem_max - mem_min) as usize,
            max_alignment: max_alignment as usize,
        })
    }

    pub unsafe fn load(&self, program_ptr: *mut u8) -> u64 {
        for header in self.reader.program_header_iter() {
            if header.segment_type() != ProgramType::Load {
                continue;
            }

            let relative_offset = header.vaddr() - self.mem_min;

            let dst = program_ptr as u64 + relative_offset.as_u64();
            let src = self.reader.buffer().as_ptr() as u64 + header.offset();
            let len = header.filesize();
            let mem_sz = header.memsize();

            unsafe {
                core::ptr::write_bytes(dst as *mut u8, 0, mem_sz as usize);
                core::ptr::copy(src as *const u8, dst as *mut u8, len as usize);
            }
        }
        self.reader.entry_point()
    }

    pub fn max_alignment(&self) -> usize {
        self.max_alignment
    }

    pub fn mem_min(&self) -> VirtAddr {
        self.mem_min
    }

    pub fn mem_max(&self) -> VirtAddr {
        self.mem_max
    }

    pub fn max_memory_needed(&self) -> usize {
        self.max_memory_needed
    }
}

impl VirtuallyReplaceable for Elf<'_> {
    fn replace<T: pager::Mapper>(&mut self, mapper: &mut pager::MapperWithVirtualAllocator<T>) {
        self.reader.replace(mapper);
    }
}

impl VirtuallyMappable for Elf<'_> {
    fn virt_map(&self, mapper: &mut impl pager::Mapper, phys_start: PhysAddr) {
        for section in self.reader.program_header_iter() {
            if section.segment_type() != ProgramType::Load {
                continue;
            }
            assert!(
                section.vaddr().as_u64() % PAGE_SIZE == 0,
                "sections need to be page aligned"
            );
            let relative_offset = (section.vaddr() - self.mem_min()).as_u64();

            // SAFETY: We know this is safe because we're parsing the elf correctly
            unsafe {
                mapper.map_to_range(
                    Page::containing_address(section.vaddr()),
                    Page::containing_address(section.vaddr() + section.memsize() - 1),
                    Frame::containing_address(phys_start + relative_offset),
                    Frame::containing_address(phys_start + relative_offset + section.memsize() - 1),
                    EntryFlags::from(section.flags()),
                )
            };
        }
    }
}

impl IdentityMappable for Elf<'_> {
    fn map(&self, mapper: &mut impl pager::Mapper) {
        self.reader.map(mapper);
    }
}
