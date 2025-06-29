use core::fmt::{Debug, Display};

use bitflags::bitflags;
use c_enum::c_enum;
use pager::{DataBuffer, EntryFlags, IdentityMappable, IdentityReplaceable, address::VirtAddr};

use crate::ElfError;

#[derive(Debug)]
pub struct ElfReader<'a> {
    buffer: DataBuffer<'a>,
}

impl<'a> ElfReader<'a> {
    pub fn new(buffer: &'a [u8]) -> Self {
        Self {
            buffer: DataBuffer::new(buffer),
        }
    }

    pub fn buffer(&self) -> &DataBuffer<'a> {
        &self.buffer
    }

    pub fn header(&self) -> ElfHeader {
        unsafe {
            core::mem::transmute(
                TryInto::<[u8; size_of::<ElfHeader>()]>::try_into(
                    &self.buffer[0..size_of::<ElfHeader>()],
                )
                .unwrap(),
            )
        }
    }

    pub fn section_name(&'a self, section: &SectionHeader) -> Result<&'a str, ElfError<'a>> {
        self.string_table_offset(section.name as usize)
    }

    pub fn section_by_name(&self, name: &str) -> Option<SectionHeader> {
        self.section_header_iter()
            .find(|e| self.section_name(e).is_ok_and(|e| e == name) && e.typ != SectionType::NULL)
    }

    pub fn section_buffer(&self, section: &SectionHeader) -> Option<&[u8]> {
        Some(&self.buffer[section.offset as usize..][..section.size as usize])
    }

    pub fn section_buffer_by_name(&self, name: &str) -> Option<&[u8]> {
        let section = self.section_by_name(name)?;
        Some(&self.buffer[section.offset as usize..][..section.size as usize])
    }

    pub fn section_link(&self, section: &SectionHeader) -> Option<SectionHeader> {
        self.section_entry(section.link as usize)
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

    pub fn string_table_offset(&'a self, offset: usize) -> Result<&'a str, ElfError<'a>> {
        let header = self.header();
        let string_table = self
            .section_entry(header.string_table_index as usize)
            .ok_or(ElfError::InvalidStringTableIndex(
                header.string_table_index as usize,
            ))?;
        let string_table = self
            .buffer
            .get(
                string_table.offset as usize
                    ..string_table.offset as usize + string_table.size as usize,
            )
            .ok_or(ElfError::InvalidStringTable)?;
        let null_terminated = &string_table[offset..];
        let end = null_terminated
            .iter()
            .position(|&b| b == 0)
            .ok_or(ElfError::InvalidStringTable)?;
        str::from_utf8(&null_terminated[..end]).map_err(|_| ElfError::InvalidStringTable)
    }

    pub fn symbol_index<T>(&self, section: &SectionHeader, index: usize) -> Option<T> {
        assert!(
            section.entry_size() != 0,
            "Section header dons't have entry size, cann't look up symbol"
        );
        assert!(
            section.entry_size() as usize >= size_of::<T>(),
            "Symbol must be less than or equal to entrysize"
        );

        let entry_count = section.size() / section.entry_size();
        if index >= entry_count as usize {
            return None;
        }

        unsafe {
            let ptr = self.section_buffer(section)?[index * section.entry_size() as usize..]
                [..section.entry_size() as usize][..size_of::<T>()]
                .as_ptr();
            let t = ptr.cast::<T>().read_unaligned();
            Some(t)
        }
    }

    pub fn program_entries_len(&self) -> usize {
        self.header().program_entries_len as usize
    }

    pub fn program_header_iter(&self) -> ProgramHeaderIter<'_> {
        ProgramHeaderIter {
            reader: self,
            index: 0,
        }
    }

    pub fn section_header_iter(&self) -> SectionHeaderIter<'_> {
        SectionHeaderIter {
            reader: self,
            index: 0,
        }
    }

    pub fn entry_point(&self) -> u64 {
        self.header().program_entry_offset
    }
}

unsafe impl IdentityReplaceable for ElfReader<'_> {
    fn identity_replace<T: pager::Mapper>(
        &mut self,
        mapper: &mut pager::MapperWithVirtualAllocator<T>,
    ) {
        self.buffer.identity_replace(mapper);
    }
}

unsafe impl IdentityMappable for ElfReader<'_> {
    fn map(&self, mapper: &mut impl pager::Mapper) {
        self.buffer.map(mapper);
    }
}

#[derive(Debug)]
pub struct ProgramHeaderIter<'a> {
    reader: &'a ElfReader<'a>,
    index: usize,
}

impl<'a> Iterator for ProgramHeaderIter<'a> {
    type Item = ProgramHeader;

    fn next(&mut self) -> Option<Self::Item> {
        self.reader.program_entry({
            let tmp = self.index;
            self.index += 1;
            tmp
        })
    }
}

impl Display for ProgramHeaderIter<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut headers = self.reader.program_header_iter();
        write!(f, "ProgramHeaders [")?;
        while let Some(header) = headers.next() {
            write!(f, "{:?},", header)?;
        }
        write!(f, "]")?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct SectionHeaderIter<'a> {
    reader: &'a ElfReader<'a>,
    index: usize,
}

impl<'a> Iterator for SectionHeaderIter<'a> {
    type Item = SectionHeader;

    fn next(&mut self) -> Option<Self::Item> {
        self.reader.section_entry({
            let tmp = self.index;
            self.index += 1;
            tmp
        })
    }
}

impl Display for SectionHeaderIter<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut headers = self.reader.program_header_iter();
        write!(f, "SectionHeaders [")?;
        while let Some(header) = headers.next() {
            write!(f, "{:?},", header)?;
        }
        write!(f, "]")?;
        Ok(())
    }
}

//  Reference from https://wiki.osdev.org/ELF
#[repr(C)]
#[derive(Debug)]
pub struct ElfHeader {
    /// Magic bytes - 0x7F, then 'ELF' in ASCII
    pub magic_bytes: [u8; 4],
    /// How many Bits???
    pub bits: ElfBits,
    /// Endian of this elf
    pub endian: ElfEndian,
    /// Header version
    pub header_version: u8,
    /// OS ABI - usually 0 for System V
    pub abi: u8,
    /// Unused (Use for padding)
    _unused: [u8; 8],
    /// Type of the elf
    pub ty: ElfType,
    /// Instruction set
    pub instruction_set: InstructionSet,
    /// Elf version (currently 1)
    pub elf_version: u32,
    /// Offset to the program entrypoint
    program_entry_offset: u64,
    /// Offset to the program headers
    program_header_table_offset: u64,
    /// Offset to the section headers
    section_header_table_offset: u64,
    /// Flags, unused in x86_64 (which we're targeting)
    flags: u32,
    /// ELF Header size
    pub header_size: u16,
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

#[derive(Debug)]
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

impl SectionHeader {
    pub fn vaddr(&self) -> u64 {
        self.vaddr
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn typ(&self) -> SectionType {
        self.typ
    }

    pub fn entry_size(&self) -> u64 {
        self.entry_size
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
        DYNSYM = 0xb
        DYNAMIC = 0x6
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

impl From<ProgramHeaderFlags> for EntryFlags {
    fn from(value: ProgramHeaderFlags) -> Self {
        let mut flags = EntryFlags::empty();

        if value.contains(ProgramHeaderFlags::Readable) {
            flags |= EntryFlags::PRESENT;
        }
        if value.contains(ProgramHeaderFlags::Writeable) {
            flags |= EntryFlags::WRITABLE;
        }
        if !value.contains(ProgramHeaderFlags::Executeable) {
            flags |= EntryFlags::NO_EXECUTE;
        }

        flags
    }
}
