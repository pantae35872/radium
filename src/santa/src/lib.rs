#![no_std]
#![feature(custom_test_frameworks)]
#![allow(dead_code)]

extern crate alloc;

use c_enum::c_enum;
use core::fmt::Debug;
use core::iter::Iterator;
use pager::{
    address::{Frame, Page, PhysAddr, VirtAddr},
    EntryFlags, IdentityMappable, VirtuallyMappable, VirtuallyReplaceable, PAGE_SIZE,
};
use reader::{ElfBits, ElfHeader, ElfReader, ProgramType, SectionType};
use sentinel::log;
use thiserror::Error;

mod reader;

#[derive(Error, Debug)]
pub enum ElfError<'a> {
    #[error("Invalid elf header")]
    InvalidHeader,
    #[error("32 Bit elf is not supported")]
    Elf32BitNotSupport,
    /// The index into the string table is not valid from the ElfHeader
    #[error("Invalid string table index {0}")]
    InvalidStringTableIndex(usize),
    /// The string table in the elf file is not valid
    #[error("Invalid string table")]
    InvalidStringTable,
    #[error("Elf magic is not valid, {magic:?}")]
    InvalidMagic { magic: [u8; 4] },
    #[error("Unknown elf relocation type {0:?}")]
    UnknownRelocationType(RelaType),
    #[error("Unable to reslove some symbol `{0}`")]
    UnresolvedSymbol(&'a str),
}

pub trait SymbolResolver {
    fn resolve(&self, symbol: &str) -> Option<u64>;
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
    pub fn new(buffer: &'a [u8]) -> Result<Self, ElfError<'a>> {
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

    pub fn apply_relocations(
        &'a self,
        base: VirtAddr,
        reslover: &impl SymbolResolver,
    ) -> Result<(), ElfError<'a>> {
        for section in self.reader.section_header_iter() {
            if section.typ() != SectionType::RELA {
                continue;
            }

            let target_section = match self.reader.section_link(&section) {
                Some(section) => section,
                None => continue,
            };

            let target_name = self.reader.section_name(&target_section);
            if !target_name.is_ok_and(|e| e == ".dynsym") {
                continue;
            }

            let rela_data = match self.reader.section_buffer(&section) {
                Some(section) => section,
                None => continue,
            };
            let count = section.size() / section.entry_size();

            for i in 0..count {
                let offset = (i * section.entry_size()) as usize;
                let rela = unsafe {
                    core::mem::transmute::<[u8; size_of::<ElfRela>()], ElfRela>(
                        rela_data[offset..offset + section.entry_size() as usize]
                            [..size_of::<ElfRela>()]
                            .try_into()
                            .unwrap(),
                    )
                };
                let offset = rela.offset - self.mem_min.as_u64();

                log!(Trace, "Relocation entry: {rela:x?}");

                match rela.typ() {
                    RelaType::X86_64_RELATIVE => {
                        unsafe {
                            *(base + offset).as_mut_ptr::<u64>() =
                                base.as_u64() + rela.addend - self.mem_min.as_u64();
                        };
                    }
                    RelaType::X86_64_64 => {
                        let sym = self
                            .reader
                            .symbol_index::<ElfSymbol>(&target_section, rela.sym() as usize)
                            .unwrap();
                        let sym = base.as_u64() + (sym.value - self.mem_min.as_u64());
                        unsafe { *(base + offset).as_mut_ptr::<u64>() = sym + rela.addend };
                    }
                    RelaType::X86_64_GLOB_DAT | RelaType::X86_64_JUMP_SLOT => {
                        let sym = self
                            .reader
                            .symbol_index::<ElfSymbol>(&target_section, rela.sym() as usize)
                            .unwrap();

                        if sym.shndx == 0 {
                            let dynstr_data = self
                                .reader
                                .section_buffer_by_name(".dynstr")
                                .ok_or(ElfError::InvalidHeader)?;

                            let end = dynstr_data[sym.name_offset as usize..]
                                .iter()
                                .position(|&c| c == 0)
                                .map(|pos| sym.name_offset as usize + pos)
                                .unwrap_or(dynstr_data.len());

                            let sym_name =
                                core::str::from_utf8(&dynstr_data[sym.name_offset as usize..end])
                                    .map_err(|_| ElfError::InvalidHeader)?;

                            let sym = reslover
                                .resolve(sym_name)
                                .ok_or(ElfError::UnresolvedSymbol(sym_name))?;
                            unsafe { *(base + offset).as_mut_ptr::<u64>() = sym };
                            continue;
                        }
                        unsafe {
                            *(base + offset).as_mut_ptr::<u64>() =
                                base.as_u64() + (sym.value - self.mem_min.as_u64())
                        };
                    }
                    t => return Err(ElfError::UnknownRelocationType(t)),
                }
            }
        }

        Ok(())
    }

    pub fn lookup_symbol(&self, name: &str, base: VirtAddr) -> Option<VirtAddr> {
        log!(Debug, "Looking up synmbol `{}`", name);

        let dynsym = self.reader.section_by_name(".dynsym")?;
        let dynstr = self.reader.section_by_name(".dynstr")?;

        log!(Trace, "Dynsym header: {dynsym:?}");
        log!(Trace, "Dynstr header: {dynstr:?}");

        let sym_count = dynsym.size() / dynsym.entry_size();

        log!(Debug, "Symbol count {sym_count}");

        let dynsym_data = self.reader.section_buffer_by_name(".dynsym")?;
        let dynstr_data = self.reader.section_buffer_by_name(".dynstr")?;

        for i in 0..sym_count {
            let sym_offset = (i * dynsym.entry_size()) as usize;
            let sym = unsafe {
                core::mem::transmute::<[u8; size_of::<ElfSymbol>()], ElfSymbol>(
                    dynsym_data[sym_offset..sym_offset + dynsym.entry_size() as usize]
                        [..size_of::<ElfSymbol>()]
                        .try_into()
                        .unwrap(),
                )
            };

            let name_offset = sym.name_offset as usize;
            if name_offset >= dynstr_data.len() {
                continue;
            }

            let end = dynstr_data[name_offset..]
                .iter()
                .position(|&c| c == 0)
                .map(|pos| name_offset + pos)
                .unwrap_or(dynstr_data.len());

            let sym_name = core::str::from_utf8(&dynstr_data[name_offset..end]).ok()?;

            if sym_name == name {
                log!(Trace, "Resloved symbol `{name}`, value: {:x}", sym.value);
                return Some(base + sym.value - self.mem_min);
            }
        }

        None
    }

    /// Load the elf file into the program ptr. without mapping with correct perrmission
    ///
    /// # Safety
    /// The caller must ensure that the provided program_ptr is valid and marked as writeable and
    /// overwriteable
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
    fn virt_map(&self, mapper: &mut impl pager::Mapper, virt_base: VirtAddr, phys_base: PhysAddr) {
        for section in self.reader.program_header_iter() {
            if section.segment_type() != ProgramType::Load {
                continue;
            }
            assert!(
                section.vaddr().as_u64() % PAGE_SIZE == 0,
                "sections need to be page aligned"
            );
            let relative_offset = (section.vaddr() - self.mem_min()).as_u64();
            let virt_start = virt_base + relative_offset;
            let virt_end = virt_start + section.memsize() - 1;
            let phys_start = phys_base + relative_offset;
            let phys_end = phys_base + relative_offset + section.memsize() - 1;

            log!(
                Trace,
                "Elf mapping [{virt_start:x}-{virt_end:x}] with {} to [{phys_start:x}-{phys_end:x}]",
                EntryFlags::from(section.flags())
            );
            // SAFETY: We know this is safe because we're parsing the elf correctly
            unsafe {
                mapper.map_to_range(
                    Page::containing_address(virt_start),
                    Page::containing_address(virt_end),
                    Frame::containing_address(phys_start),
                    Frame::containing_address(phys_end),
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

#[derive(Debug)]
#[repr(C)]
struct ElfSymbol {
    name_offset: u32,
    info: u8,
    other: u8,
    shndx: u16,
    value: u64,
    size: u64,
}

#[derive(Debug)]
#[repr(C)]
struct ElfRela {
    offset: u64,
    info: u64,
    addend: u64,
}

c_enum! {
    pub enum RelaType: u64 {
        X86_64_RELATIVE = 8
        X86_64_64 = 1
        X86_64_GLOB_DAT = 6
        X86_64_JUMP_SLOT = 7
    }
}

impl ElfRela {
    fn typ(&self) -> RelaType {
        RelaType(self.info & 0xffffffff)
    }

    fn sym(&self) -> u64 {
        self.info >> 32
    }
}
