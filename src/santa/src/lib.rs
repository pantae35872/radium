#![no_std]
#![feature(custom_test_frameworks)]

use bitflags::bitflags;
use c_enum::c_enum;
use core::iter::Iterator;
use core::option::Option;

// TODO: Add testing
// TODO: Proper debug print
#[derive(Debug)]
pub struct Elf<'a> {
    buffer: &'a [u8],
}

//  Reference from https://wiki.osdev.org/ELF
#[repr(C)]
struct ElfHeader {
    magic_bytes: [u8; 4],             // Magic bytes - 0x7F, then 'ELF' in ASCII
    bits: ElfBits,                    // How many Bits???
    endian: ElfEndian,                // Endian of this elf
    header_version: u8,               // Header version
    abi: u8,                          // OS ABI - usually 0 for System V
    _unused: [u8; 8],                 // Unused (Use for padding)
    ty: ElfType,                      // Type of the elf
    instruction_set: InstructionSet,  // Instruction set
    elf_version: u32,                 // Elf version (currently 1)
    program_entry_offset: u64,        // Offset to the program entrypoint
    program_header_table_offset: u64, // Offset to the program headers
    section_header_table_offset: u64, // Offset to the section headers
    flags: u32,                       // Flags, unused in x86_64 (which we're targeting)
    header_size: u16,                 // ELF Header size
    program_entry_size: u16,          // Size of an entry in the program header table
    program_entries_len: u16,         // Number of entries in the program header table
    section_entry_size: u16,          // Size of an entry in the section header table
    section_entries_len: u16,         // Number of entries in the section header table
    section_index: u16,               // Section index to the section header string table
}

#[repr(C)]
pub struct ProgramHeader {
    program_type: ProgramType,
    flags: ProgramHeaderFlags,
    program_offset: u64, // The offset in the file that the data for this segment can be found (p_offset)
    program_vaddr: u64,  // Where you should start to put this segment in virtual memory (p_vaddr)
    reserved: u64,       // Reserved for segment's physical address (p_paddr)
    program_filesize: u64, // Size of the segment in the file (p_filesz)
    program_memsize: u64, // Size of the segment in memory (p_memsz, at least as big as p_filesz)
    alignment: u64,      //The required alignment for this section (usually a power of 2)
}

#[repr(C)]
pub struct SectionHeader {
    name: u32,                 // Offset to the section name in the section header string table.
    typ: SectionType,          // SectionType
    flags: SectionHeaderFlags, // Flags
    vaddr: u64,                // Virtual address where the section should be loaded in memory.
    offset: u64,               // Offset of the section's data in the file.
    size: u64,                 // Size of the section in bytes
    link: u32,                 // Section index of an associated section.
    info: u32,                 // Extra information; interpretation depends on the section type.
    addralign: u64,            // Address alignment constraints for the section.
    entry_size: u64, // Size of each entry if the section holds a table of fixed-size entries
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
    #[derive(Clone, Copy)]
    pub struct ProgramHeaderFlags: u32 {
        const Executeable = 0x1;
        const Writeable = 0x2;
        const Readable = 0x4;
    }
    #[derive(Clone, Copy)]
    pub struct SectionHeaderFlags: u64 {
        const Writeable = 0x1;
        const Alloc = 0x2;
        const Executeable = 0x4;
    }
}

#[derive(Debug)]
pub enum ElfError {
    InvalidHeader,
    InvalidMagic { magic: [u8; 4] },
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
        let s = Self { buffer };
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

    pub fn vaddr(&self) -> u64 {
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
