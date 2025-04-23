#![no_std]
#![feature(custom_test_frameworks)]

extern crate alloc;

use alloc::vec::Vec;
use bitflags::bitflags;
use c_enum::c_enum;
use core::ffi::CStr;
use core::fmt::Debug;
use core::iter::Iterator;
use core::option::Option;
use pager::{
    address::{Frame, PhysAddr, VirtAddr},
    DataBuffer, EntryFlags, IdentityMappable, PAGE_SIZE,
};

// TODO: Add testing
pub struct Elf<'a> {
    buffer: DataBuffer<'a>,
}

//  Reference from https://wiki.osdev.org/ELF
#[repr(C)]
#[derive(Debug)]
struct ElfHeader {
    /// Magic bytes - 0x7F, then 'ELF' in ASCII
    magic_bytes: [u8; 4],
    /// How many Bits???
    bits: ElfBits,
    /// Endian of this elf
    endian: ElfEndian,
    /// Header version
    header_version: u8,
    /// OS ABI - usually 0 for System V
    abi: u8,
    /// Unused (Use for padding)
    _unused: [u8; 8],
    /// Type of the elf
    ty: ElfType,
    /// Instruction set
    instruction_set: InstructionSet,
    /// Elf version (currently 1)
    elf_version: u32,
    /// Offset to the program entrypoint
    program_entry_offset: u64,
    /// Offset to the program headers
    program_header_table_offset: u64,
    /// Offset to the section headers
    section_header_table_offset: u64,
    /// Flags, unused in x86_64 (which we're targeting)
    flags: u32,
    /// ELF Header size
    header_size: u16,
    /// Size of each entry in the program header table
    program_entry_size: u16,
    /// Number of entries in the program header table
    program_entries_len: u16,
    /// Size of each entry in the section header table
    section_entry_size: u16,
    /// Number of entries in the section header table
    section_entries_len: u16,
    /// Section index to the section header string table
    string_table_index: u16,
}

#[repr(C)]
#[derive(Debug)]
pub struct ProgramHeader {
    program_type: ProgramType,
    flags: ProgramHeaderFlags,
    /// The offset in the file that the data for this segment can be found (p_offset)
    program_offset: u64,
    /// Where you should start to put this segment in virtual memory (p_vaddr)
    program_vaddr: VirtAddr,
    /// Reserved for segment's physical address (p_paddr)
    reserved: u64,
    /// Size of the segment in the file (p_filesz)
    program_filesize: u64,
    /// Size of the segment in memory (p_memsz, at least as big as p_filesz)
    program_memsize: u64,
    /// The required alignment for this section (usually a power of 2)
    alignment: u64,
}

#[repr(C)]
pub struct SectionHeader {
    // Offset to the section name in the section header string table.
    name: u32,
    /// Section type
    typ: SectionType,
    flags: SectionHeaderFlags,
    /// Virtual address where the section should be loaded in memory.
    vaddr: u64,
    /// Offset of the section's data in the file.
    offset: u64,
    /// Size of the section in bytes
    size: u64,
    /// Section index of an associated section.
    link: u32,
    /// Extra information; interpretation depends on the section type.
    info: u32,
    /// Address alignment constraints for the section.
    addralign: u64,
    /// Size of each entry if the section holds a table of fixed-size entries
    entry_size: u64,
}
c_enum! {
    pub(crate) enum ElfBits: u8 {
        B32 = 1
        B64 = 2
    }

    pub(crate) enum ElfEndian: u8 {
        LittleEndian = 1
        BigEndian = 2
    }

    pub enum ElfType: u16 {
        Relocatable = 1
        Executable = 2
        Shared = 3
        Core = 4
    }

    pub enum InstructionSet: u16 {
        NoSpecific  =	 0x00
        Sparc 	    =    0x02
        x86 	    =    0x03
        MIPS 	    =    0x08
        PowerPC     =    0x14
        ARM 	    =    0x28
        SuperH 	    =    0x2A
        IA_64 	    =    0x32
        x86_64 	    =    0x3E
        AArch64     =    0xB7
        RISC_V 	    =    0xF3
    }

    // We don't care about processor specific stuff
    pub enum ProgramType: u32 {
        Null = 0
        Load = 1
        Dynamic = 2
        Interp = 3
        Note = 4
        SHLIB = 5 // What ever the fuck this is
        PHDR = 6
    }

    pub enum SectionType: u32 {
        NULL = 0
        PROGBITS = 1
        SYMTAB = 2
        STRTAB = 3
        RELA = 4
        NOBITS = 8
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct ProgramHeaderFlags: u32 {
        const Executeable = 0x1;
        const Writeable = 0x2;
        const Readable = 0x4;
    }
    #[derive(Clone, Copy, Debug)]
    pub struct SectionHeaderFlags: u64 {
        const Writeable = 0x1;
        const Alloc = 0x2;
        const Executeable = 0x4;
    }
}

#[derive(Debug)]
pub enum ElfError {
    InvalidHeader,
    /// The index into the string table is not valid from the ElfHeader
    InvalidStringTableIndex(usize),
    /// The string table in the elf file is not valid
    InvalidStringTable,
    InvalidMagic {
        magic: [u8; 4],
    },
}

pub struct ProgramHeaderIter<'a> {
    elf: &'a Elf<'a>,
    index: usize,
}

pub struct SectionHeaderIter<'a> {
    elf: &'a Elf<'a>,
    index: usize,
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
        let s = Self {
            buffer: DataBuffer::new(buffer),
        };
        if s.header().bits != ElfBits::B64 {
            return Err(ElfError::InvalidHeader);
        }
        Ok(s)
    }

    fn header(&self) -> ElfHeader {
        unsafe {
            core::mem::transmute(
                TryInto::<[u8; size_of::<ElfHeader>()]>::try_into(
                    &self.buffer[0..size_of::<ElfHeader>()],
                )
                .unwrap(),
            )
        }
    }

    pub fn section_entry(&self, index: usize) -> Option<SectionHeader> {
        let header = self.header();
        if index >= header.section_entries_len as usize {
            return None;
        }
        let offset = header.section_header_table_offset as usize
            + index * header.section_entry_size as usize;
        self.buffer
            .get(offset..(offset + header.section_entry_size as usize))
            .and_then(|buf| unsafe {
                Some(core::mem::transmute::<
                    [u8; size_of::<SectionHeader>()],
                    SectionHeader,
                >(
                    // We slice the buf just in case that the program_entry_size is larger than
                    TryInto::<[u8; size_of::<SectionHeader>()]>::try_into(
                        buf.get(0..size_of::<SectionHeader>())?, // Use get bc if somehow the
                                                                 // program_entry_size is less
                                                                 // size_of::<ElfProgramHeader>()
                    )
                    .unwrap(),
                ))
            })
    }

    pub fn program_entry(&self, index: usize) -> Option<ProgramHeader> {
        let header = self.header();
        if index >= header.program_entries_len as usize {
            return None;
        }
        let offset = header.program_header_table_offset as usize
            + index * header.program_entry_size as usize;
        self.buffer
            .get(offset..(offset + header.program_entry_size as usize))
            .and_then(|buf| unsafe {
                Some(core::mem::transmute::<
                    [u8; size_of::<ProgramHeader>()],
                    ProgramHeader,
                >(
                    // We slice the buf just in case that the program_entry_size is larger than
                    TryInto::<[u8; size_of::<ProgramHeader>()]>::try_into(
                        buf.get(0..size_of::<ProgramHeader>())?, // Use get bc if somehow the
                                                                 // program_entry_size is less
                                                                 // size_of::<ElfProgramHeader>()
                    )
                    .unwrap(),
                ))
            })
    }

    pub fn string_table_index(&self, _index: usize) -> Result<&CStr, ElfError> {
        let header = self.header();
        let string_table = self
            .section_entry(header.string_table_index as usize)
            .ok_or(ElfError::InvalidStringTableIndex(
                header.string_table_index as usize,
            ))?;
        let _string_table = self
            .buffer
            .get(
                string_table.offset as usize
                    ..string_table.offset as usize + string_table.size as usize,
            )
            .ok_or(ElfError::InvalidStringTable)?;
        todo!("Use somesort of buffer because we don't want to iterate through a cstring, it's ineffecient")
    }

    pub fn program_entries_len(&self) -> usize {
        self.header().program_entries_len as usize
    }

    pub fn program_header_iter(&self) -> ProgramHeaderIter {
        ProgramHeaderIter {
            elf: self,
            index: 0,
        }
    }

    pub fn section_header_iter(&self) -> SectionHeaderIter {
        SectionHeaderIter {
            elf: self,
            index: 0,
        }
    }

    pub fn entry_point(&self) -> u64 {
        self.header().program_entry_offset
    }
}

impl IdentityMappable for Elf<'_> {
    fn map(&self, mapper: &mut impl pager::Mapper) {
        for section in self.program_header_iter() {
            if section.segment_type() != ProgramType::Load {
                continue;
            }
            assert!(
                section.vaddr().as_u64() % PAGE_SIZE == 0,
                "sections need to be page aligned"
            );

            // SAFETY: We know this is safe because we're parsing the elf correctly
            unsafe {
                mapper.identity_map_range(
                    Frame::containing_address(PhysAddr::new(section.vaddr().as_u64())),
                    Frame::containing_address(PhysAddr::new(
                        section.vaddr().as_u64() + section.memsize() - 1,
                    )),
                    EntryFlags::from_elf_program_flags(&section.flags()),
                )
            };
        }
        self.buffer.map(mapper);
    }
}

trait FromHeaderFlags {
    fn from_elf_program_flags(section: &ProgramHeaderFlags) -> EntryFlags;
}

impl FromHeaderFlags for EntryFlags {
    fn from_elf_program_flags(section: &ProgramHeaderFlags) -> EntryFlags {
        let mut flags = EntryFlags::empty();

        if section.contains(ProgramHeaderFlags::Readable) {
            flags |= EntryFlags::PRESENT;
        }
        if section.contains(ProgramHeaderFlags::Writeable) {
            flags |= EntryFlags::WRITABLE;
        }
        if !section.contains(ProgramHeaderFlags::Executeable) {
            flags |= EntryFlags::NO_EXECUTE;
        }

        flags
    }
}

impl ProgramHeader {
    pub fn segment_type(&self) -> ProgramType {
        self.program_type
    }

    pub fn flags(&self) -> ProgramHeaderFlags {
        self.flags
    }

    pub fn offset(&self) -> u64 {
        self.program_offset
    }

    pub fn vaddr(&self) -> VirtAddr {
        self.program_vaddr
    }

    pub fn filesize(&self) -> u64 {
        self.program_filesize
    }

    pub fn memsize(&self) -> u64 {
        self.program_memsize
    }

    pub fn alignment(&self) -> u64 {
        self.alignment
    }
}

impl SectionHeader {
    pub fn vaddr(&self) -> u64 {
        self.vaddr
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn flags(&self) -> SectionHeaderFlags {
        self.flags
    }

    pub fn alignment(&self) -> u64 {
        self.addralign
    }
}

impl Debug for Elf<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Elf {{ {header:?}, ProgramHeaders: {program_headers:?}, SectionHeaders: {section_headers:?} }}",
            header = self.header(),
            program_headers = self.program_header_iter().collect::<Vec<_>>(),
            section_headers = self.section_header_iter().collect::<Vec<_>>()
        )
    }
}

impl Debug for SectionHeader {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "SectionHeader: {{ name: }}")
    }
}

impl<'a> Iterator for ProgramHeaderIter<'a> {
    type Item = ProgramHeader;

    fn next(&mut self) -> Option<Self::Item> {
        self.elf.program_entry({
            let tmp = self.index;
            self.index += 1;
            tmp
        })
    }
}

impl<'a> Iterator for SectionHeaderIter<'a> {
    type Item = SectionHeader;

    fn next(&mut self) -> Option<Self::Item> {
        self.elf.section_entry({
            let tmp = self.index;
            self.index += 1;
            tmp
        })
    }
}
