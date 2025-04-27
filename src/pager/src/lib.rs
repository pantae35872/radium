#![no_std]
#![feature(pointer_is_aligned_to)]
#![feature(ptr_internals)]
#![allow(internal_features)]

use core::{fmt::Display, ops::Deref};

use address::{Frame, Page, PhysAddr, VirtAddr};
use bitflags::bitflags;

pub mod address;
pub mod allocator;
pub mod gdt;
pub mod paging;
pub mod registers;

pub const PAGE_SIZE: u64 = 4096;

/// The kernel uses higher half canonical address as it's address space
/// heres are the visualization
///
/// Canonical high addresses: 0xFFFF_8000_0000_0000 -> 0xFFFF_FFFF_FFFF_FFFF
/// +----------------------------+ 0xFFFF_8000_0000_0000
/// | Kernel ELF                 |
/// +----------------------------+ 0xFFFF_8000_FFFF_FFFF
///
/// +----------------------------+ 0xFFFF_9000_0000_0000
/// | Direct Physical Map        |
/// | (1:1 virtual -> physical)  |
/// +----------------------------+ 0xFFFF_A000_0000_0000
///
/// +----------------------------+ 0xFFFF_B000_0000_0000
/// | General Kernel Use         |
/// | (dynamic memory, stack,    |
/// |  kernel heap, etc)         |
/// +----------------------------+ 0xFFFF_FFFF_FFFF_FFFF
pub const KERNEL_START: VirtAddr = VirtAddr::canonical_higher_half();
pub const KERNEL_DIRECT_PHYSICAL_MAP: VirtAddr = VirtAddr::new(0xFFFF_9000_0000_0000);
pub const KERNEL_GENERAL_USE: VirtAddr = VirtAddr::new(0xFFFF_B000_0000_0000);

pub trait Mapper {
    unsafe fn identity_map_range(
        &mut self,
        start_frame: Frame,
        end_frame: Frame,
        entry_flags: EntryFlags,
    );

    fn map_range(&mut self, start_page: Page, end_page: Page, flags: EntryFlags);

    unsafe fn identity_map(&mut self, frame: Frame, flags: EntryFlags);

    unsafe fn identity_map_by_size(&mut self, start_frame: Frame, size: usize, flags: EntryFlags) {
        unsafe {
            self.identity_map_range(
                start_frame,
                Frame::containing_address(start_frame.start_address() + size - 1),
                flags,
            )
        };
    }

    unsafe fn map_to_range_by_size(
        &mut self,
        start_page: Page,
        start_frame: Frame,
        size: usize,
        flags: EntryFlags,
    ) {
        unsafe {
            self.map_to_range(
                start_page,
                Page::containing_address(start_page.start_address() + size - 1),
                start_frame,
                Frame::containing_address(start_frame.start_address() + size - 1),
                flags,
            )
        };
    }

    unsafe fn map_to_range(
        &mut self,
        start_page: Page,
        end_page: Page,
        start_frame: Frame,
        end_frame: Frame,
        flags: EntryFlags,
    );

    unsafe fn unmap_addr(&mut self, page: Page) -> Frame;
}

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct EntryFlags: u64 {
        const PRESENT =         1 << 0;
        const WRITABLE =        1 << 1;
        const USER_ACCESSIBLE = 1 << 2;
        const WRITE_THROUGH =   1 << 3;
        const NO_CACHE =        1 << 4;
        const ACCESSED =        1 << 5;
        const DIRTY =           1 << 6;
        const HUGE_PAGE =       1 << 7;
        const GLOBAL =          1 << 8;
        const OVERWRITEABLE =   1 << 10; // Custom flags. This flags mean the mapped address can be
                                         // overwrite when mapping
        const NO_EXECUTE =      1 << 63;
    }
}

pub trait IdentityMappable {
    fn map(&self, mapper: &mut impl Mapper);
}

pub trait VirtuallyMappable {
    fn virt_map(&self, mapper: &mut impl Mapper, phys_start: PhysAddr);
}

impl Display for EntryFlags {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "flag: {}", self.0)
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct DataBuffer<'a> {
    buffer: &'a [u8],
}

impl<'a> DataBuffer<'a> {
    pub unsafe fn from_raw() -> Self {
        todo!()
    }

    pub fn new(buffer: &'a [u8]) -> Self {
        Self { buffer }
    }

    pub fn buffer(&self) -> &'a [u8] {
        self.buffer
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u16)]
pub enum PrivilegeLevel {
    Ring0 = 0,
    Ring1 = 1,
    Ring2 = 2,
    Ring3 = 3,
}

impl PrivilegeLevel {
    /// Create privilege level from u16 and truncate the upper bits
    pub fn from_u16_truncate(level: u16) -> Self {
        let level = level & 0b11;
        match level {
            0 => Self::Ring0,
            1 => Self::Ring1,
            2 => Self::Ring2,
            3 => Self::Ring3,
            _ => unreachable!(),
        }
    }

    pub fn as_u16(&self) -> u16 {
        *self as u16
    }
}

impl<'a> Deref for DataBuffer<'a> {
    type Target = [u8];

    fn deref(&self) -> &'a Self::Target {
        self.buffer
    }
}

impl IdentityMappable for DataBuffer<'_> {
    fn map(&self, mapper: &mut impl Mapper) {
        let buf_start = PhysAddr::new(self.buffer as *const [u8] as *const u8 as u64);
        let buf_end = PhysAddr::new(buf_start.as_u64() + self.buffer.len() as u64 - 1);
        // SAFETY: We know this is safe if created correctly
        unsafe {
            mapper.identity_map_range(buf_start.into(), buf_end.into(), EntryFlags::NO_EXECUTE)
        };
    }
}
