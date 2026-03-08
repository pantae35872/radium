#![no_std]
#![feature(pointer_is_aligned_to)]
#![feature(ptr_internals)]
#![allow(internal_features)]

use core::{fmt::Display, ops::Deref, panic::Location};

use address::{Page, PhysAddr, VirtAddr};
use alloc::vec::Vec;
use allocator::virt_allocator::VirtualAllocator;
use bitflags::bitflags;
use sentinel::log;

use crate::address::PageSize;

extern crate alloc;

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
/// +----------------------------+ 0xFFFF_F000_0000_0000
///
/// +----------------------------+ 0xFFFF_FE00_0000_0000
/// | Recursive Mapping          |
/// +----------------------------+ 0xFFFF_FFFF_FFFF_FFFF
pub const KERNEL_START: VirtAddr = VirtAddr::canonical_higher_half();
pub const KERNEL_DIRECT_PHYSICAL_MAP: VirtAddr = VirtAddr::new(0xFFFF_9000_0000_0000);
pub const KERNEL_GENERAL_USE: VirtAddr = VirtAddr::new(0xFFFF_B000_0000_0000);

static GENERAL_VIRTUAL_ALLOCATOR: VirtualAllocator = VirtualAllocator::new(
    KERNEL_GENERAL_USE,
    (VirtAddr::new(0xFFFF_F000_0000_0000).as_u64() - KERNEL_GENERAL_USE.as_u64()) as usize,
);

#[track_caller]
pub fn virt_addr_alloc<S: PageSize>(size_in_pages: u64) -> Page<S> {
    let allocated = GENERAL_VIRTUAL_ALLOCATOR.allocate(size_in_pages as usize).expect("RAN OUT OF VIRTUAL ADDR");
    log!(
        Debug,
        "\"{}\" Called virt_addr_alloc with size {size_in_pages}, giving {:x}-{:x}",
        Location::caller(),
        allocated.start_address(),
        allocated.start_address() + size_in_pages * S::SIZE
    );
    allocated
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

impl Display for DataBuffer<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let buf_start = PhysAddr::new(self.buffer as *const [u8] as *const u8 as u64);
        let buf_end = PhysAddr::new(buf_start.as_u64() + self.buffer.len() as u64 - 1);

        write!(f, "[{:#x}-{:#x}]", buf_start, buf_end)
    }
}

impl Clone for DataBuffer<'_> {
    fn clone(&self) -> Self {
        Self { buffer: Vec::leak(self.buffer.to_vec()) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageLevel {
    Page4K, // 4 KiB pages
    Page2M, // 2 MiB pages (huge)
    Page1G, // 1 GiB pages (huge)
}
