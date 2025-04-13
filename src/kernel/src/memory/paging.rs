pub use self::entry::*;
use self::mapper::Mapper;
use self::table::{RecurseLevel4, Table};
use self::temporary_page::TemporaryPage;
use crate::memory::{Frame, FrameAllocator, PAGE_SIZE};
use crate::{hlt_loop, log, serial_println};
use bootbridge::BootBridge;
use core::fmt::Display;
use core::ops::{Add, Deref, DerefMut};
use core::ptr::Unique;
use santa::SectionHeaderFlags;
use table::{
    DirectLevel4, DirectP4Create, HierarchicalLevel, NextTableAddress, RecurseP4Create, TableLevel,
    TableLevel4,
};
use x86_64::registers::control::{self, Cr3, Cr3Flags};
use x86_64::structures::paging::PhysFrame;
use x86_64::{PhysAddr, VirtAddr};

use super::allocator::linear_allocator::LinearAllocator;

mod entry;
mod mapper;
pub mod table;
mod temporary_page;

const ENTRY_COUNT: u64 = 512;

bitflags! {
    #[derive(Clone, Copy, Debug)]
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
        const OVERWRITEABLE =   1 << 62; // Custom flags. This flags mean the mapped address can be
                                         // overwrite when mapping
        const NO_EXECUTE =      1 << 63;
    }
}

impl EntryFlags {
    pub fn from_elf_section_flags(section: &SectionHeaderFlags) -> EntryFlags {
        let mut flags = EntryFlags::empty();

        if section.contains(SectionHeaderFlags::Alloc) {
            flags |= EntryFlags::PRESENT;
        }
        if section.contains(SectionHeaderFlags::Writeable) {
            flags |= EntryFlags::WRITABLE;
        }
        if !section.contains(SectionHeaderFlags::Executeable) {
            flags |= EntryFlags::NO_EXECUTE;
        }

        flags
    }
}

impl Display for EntryFlags {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "flag: {}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Page {
    number: u64,
}

impl Page {
    pub fn containing_address(address: u64) -> Page {
        Page {
            number: address / PAGE_SIZE,
        }
    }

    pub fn start_address(&self) -> u64 {
        self.number * PAGE_SIZE
    }

    fn p4_index(&self) -> u64 {
        (self.number >> 27) & 0o777
    }
    fn p3_index(&self) -> u64 {
        (self.number >> 18) & 0o777
    }
    fn p2_index(&self) -> u64 {
        (self.number >> 9) & 0o777
    }
    fn p1_index(&self) -> u64 {
        (self.number >> 0) & 0o777
    }

    pub fn range_inclusive(start: Page, end: Page) -> PageIter {
        PageIter { start, end }
    }
}
impl Add<u64> for Page {
    type Output = Page;

    fn add(self, rhs: u64) -> Page {
        Page {
            number: self.number + rhs,
        }
    }
}

#[derive(Clone)]
pub struct PageIter {
    start: Page,
    end: Page,
}

impl Iterator for PageIter {
    type Item = Page;

    fn next(&mut self) -> Option<Page> {
        if self.start <= self.end {
            let page = self.start;
            self.start.number += 1;
            Some(page)
        } else {
            None
        }
    }
}

pub struct ActivePageTable<P4: TableLevel4> {
    p4: Unique<Table<P4>>,
    mapper: Mapper<P4>,
}

impl<P4> ActivePageTable<P4>
where
    P4: TableLevel4,
    P4::CreateMarker: RecurseP4Create<P4>,
{
    pub unsafe fn new() -> ActivePageTable<P4> {
        ActivePageTable {
            p4: P4::CreateMarker::create(),
            mapper: Mapper::new(),
        }
    }
}

impl<P4> ActivePageTable<P4>
where
    P4: TableLevel4,
    P4::CreateMarker: DirectP4Create<P4>,
{
    pub unsafe fn new_custom(p4: *mut Table<P4>) -> ActivePageTable<P4> {
        ActivePageTable {
            p4: P4::CreateMarker::create(p4),
            mapper: Mapper::new_custom(p4),
        }
    }
}

impl<P4> Deref for ActivePageTable<P4>
where
    P4: TableLevel4,
{
    type Target = Mapper<P4>;

    fn deref(&self) -> &Mapper<P4> {
        &self.mapper
    }
}

impl<P4> DerefMut for ActivePageTable<P4>
where
    P4: TableLevel4,
{
    fn deref_mut(&mut self) -> &mut Mapper<P4> {
        &mut self.mapper
    }
}

impl<P4> ActivePageTable<P4>
where
    P4: HierarchicalLevel + TableLevel4,
    P4::Marker: NextTableAddress,
    P4::NextLevel: HierarchicalLevel,
    <<P4 as HierarchicalLevel>::NextLevel as TableLevel>::Marker: NextTableAddress,
    <<P4 as HierarchicalLevel>::NextLevel as HierarchicalLevel>::NextLevel: HierarchicalLevel,
    <<<P4 as HierarchicalLevel>::NextLevel as HierarchicalLevel>::NextLevel as TableLevel>::Marker:
        NextTableAddress,
{
    fn p4_mut(&mut self) -> &mut Table<P4> {
        unsafe { self.p4.as_mut() }
    }

    pub fn with<F>(
        &mut self,
        table: &mut InactivePageTable,
        temporary_page: &mut temporary_page::TemporaryPage,
        f: F,
    ) where
        F: FnOnce(&mut Mapper<P4>),
    {
        use x86_64::instructions::tlb;
        {
            let (level_4_table_frame, _) = control::Cr3::read();
            let backup = Frame::containing_address(level_4_table_frame.start_address().as_u64());

            let p4_table = temporary_page.map_table_frame(backup.clone(), self);

            self.p4_mut()[511].set(
                table.p4_frame.clone(),
                EntryFlags::PRESENT | EntryFlags::WRITABLE,
            );
            tlb::flush_all();

            f(self);

            p4_table[511].set(backup, EntryFlags::PRESENT | EntryFlags::WRITABLE);
            tlb::flush_all();
        }

        temporary_page.unmap(self);
    }

    pub fn switch(&mut self, new_table: InactivePageTable) -> InactivePageTable {
        let (level_4_table_frame, _) = control::Cr3::read();
        let old_table = InactivePageTable {
            p4_frame: Frame::containing_address(level_4_table_frame.start_address().as_u64()),
        };
        let frame = PhysFrame::from_start_address(new_table.p4_frame.start_address())
            .expect("Failed to cr3 frame");
        unsafe {
            Cr3::write(frame, Cr3Flags::PAGE_LEVEL_WRITETHROUGH);
        }
        old_table
    }

    fn p4(&self) -> &Table<P4> {
        unsafe { self.p4.as_ref() }
    }

    pub fn translate(&self, virtual_address: VirtAddr) -> Option<PhysAddr> {
        let offset = virtual_address.as_u64() % PAGE_SIZE;
        return self
            .translate_page(Page::containing_address(virtual_address.as_u64()))
            .map(|frame| PhysAddr::new(frame.number * PAGE_SIZE + offset));
    }

    fn translate_page(&self, page: Page) -> Option<Frame> {
        let p3 = self.p4().next_table(page.p4_index());

        let huge_page = || {
            p3.and_then(|p3| {
                let p3_entry = &p3[page.p3_index() as usize];
                if let Some(start_frame) = p3_entry.pointed_frame() {
                    if p3_entry.flags().contains(EntryFlags::HUGE_PAGE) {
                        assert!(start_frame.number % (ENTRY_COUNT * ENTRY_COUNT) == 0);
                        return Some(Frame {
                            number: start_frame.number
                                + page.p2_index() * ENTRY_COUNT
                                + page.p1_index(),
                        });
                    }
                }
                if let Some(p2) = p3.next_table(page.p3_index()) {
                    let p2_entry = &p2[page.p2_index() as usize];
                    if let Some(start_frame) = p2_entry.pointed_frame() {
                        if p2_entry.flags().contains(EntryFlags::HUGE_PAGE) {
                            assert!(start_frame.number % ENTRY_COUNT == 0);
                            return Some(Frame {
                                number: start_frame.number + page.p1_index(),
                            });
                        }
                    }
                }
                None
            })
        };

        p3.and_then(|p3| p3.next_table(page.p3_index()))
            .and_then(|p2| p2.next_table(page.p2_index()))
            .and_then(|p1| p1[page.p1_index() as usize].pointed_frame())
            .or_else(huge_page)
    }
}

pub struct InactivePageTable {
    p4_frame: Frame,
}

impl InactivePageTable {
    pub fn new<P4>(
        frame: Frame,
        active_table: &mut ActivePageTable<P4>,
        temporary_page: &mut TemporaryPage,
    ) -> InactivePageTable
    where
        P4: HierarchicalLevel + TableLevel4,
        P4::Marker: NextTableAddress,
        P4::NextLevel: HierarchicalLevel,
        <<P4 as HierarchicalLevel>::NextLevel as TableLevel>::Marker: NextTableAddress,
        <<P4 as HierarchicalLevel>::NextLevel as HierarchicalLevel>::NextLevel: HierarchicalLevel,
        <<<P4 as HierarchicalLevel>::NextLevel as HierarchicalLevel>::NextLevel as TableLevel>::Marker:
            NextTableAddress,
    {
        {
            let table = temporary_page.map_table_frame(frame.clone(), active_table);
            table.zero();
            table[511].set(frame.clone(), EntryFlags::PRESENT | EntryFlags::WRITABLE);
        }
        temporary_page.unmap(active_table);

        InactivePageTable { p4_frame: frame }
    }

    pub unsafe fn from_raw_frame(frame: Frame) -> InactivePageTable {
        InactivePageTable { p4_frame: frame }
    }
}

pub unsafe fn early_map_kernel<A>(
    bootbridge: &BootBridge,
    allocator: &mut A,
    linear_allocator: &LinearAllocator,
) where
    A: FrameAllocator,
{
    let p4_table = allocator
        .allocate_frame()
        .expect("Failed to allocate frame for temporary early boot");
    let mut active_table = ActivePageTable::<DirectLevel4>::new_custom(
        p4_table.start_address().as_u64() as *mut Table<DirectLevel4>,
    );
    bootbridge.kernel_elf().map_self(|start, end, flags| {
        active_table.identity_map_range(
            start.into(),
            end.into(),
            EntryFlags::from_elf_section_flags(&flags),
            allocator,
        )
    });

    // Map the boot-info
    bootbridge.map_self(|start, size| {
        let start_frame = Frame::containing_address(start);
        let end_frame = Frame::containing_address(start + size - 1);
        for frame in Frame::range_inclusive(start_frame, end_frame) {
            active_table.identity_map(
                frame,
                EntryFlags::PRESENT | EntryFlags::WRITABLE | EntryFlags::OVERWRITEABLE,
                allocator,
            );
        }
    });

    // Do a recursive map
    active_table.p4_mut()[511] = Entry(
        p4_table.start_address().as_u64() | (EntryFlags::PRESENT | EntryFlags::WRITABLE).bits(),
    );

    // Map the allocator memories (used by the next boot strap process)
    active_table.identity_map_range(
        (linear_allocator.original_start() as u64).into(),
        Frame::containing_address(
            linear_allocator.original_start() as u64 + linear_allocator.size() as u64 - 1,
        ),
        EntryFlags::PRESENT | EntryFlags::WRITABLE,
        allocator,
    );

    // Unsafely change the cr3 bc we have no recursive map on the uefi table
    // TODO: If we want to do this safely and by design, we need huge pages support for both L3 and
    // L2 huge pages bc we can't work with the uefi table without huge pages support
    Cr3::write(
        PhysFrame::from_start_address(p4_table.start_address()).unwrap(),
        Cr3Flags::PAGE_LEVEL_WRITETHROUGH,
    );
}

pub fn create_mappings<F, A>(f: F, allocator: &mut A) -> InactivePageTable
where
    F: FnOnce(&mut Mapper<RecurseLevel4>, &mut A),
    A: FrameAllocator,
{
    let mut temporary_page = TemporaryPage::new(Page { number: 0xdeadbeef }, allocator);
    let mut active_table = unsafe { ActivePageTable::<RecurseLevel4>::new() };

    let mut new_table = {
        let frame = allocator.allocate_frame().expect("no more frames");
        InactivePageTable::new(frame, &mut active_table, &mut temporary_page)
    };

    active_table.with(&mut new_table, &mut temporary_page, |mapper| {
        f(mapper, allocator);
    });

    new_table
}

pub fn remap_the_kernel<A>(
    allocator: &mut A,
    bootbridge: &BootBridge,
) -> ActivePageTable<RecurseLevel4>
where
    A: FrameAllocator,
{
    let mut temporary_page = TemporaryPage::new(Page { number: 0xdeadbeaf }, allocator);

    let mut active_table = unsafe { ActivePageTable::new() };
    let mut new_table = {
        let frame = allocator.allocate_frame().expect("no more frames");
        InactivePageTable::new(frame, &mut active_table, &mut temporary_page)
    };
    active_table.with(&mut new_table, &mut temporary_page, |mapper| {
        bootbridge.kernel_elf().map_self(|start, end, flags| {
            mapper.identity_map_range(
                start.into(),
                end.into(),
                EntryFlags::from_elf_section_flags(&flags),
                allocator,
            )
        });

        bootbridge.map_self(|start, size| {
            let start_frame = Frame::containing_address(start);
            let end_frame = Frame::containing_address(start + size - 1);
            for frame in Frame::range_inclusive(start_frame, end_frame) {
                mapper.identity_map(
                    frame,
                    EntryFlags::PRESENT | EntryFlags::WRITABLE | EntryFlags::OVERWRITEABLE,
                    allocator,
                );
            }
        });
    });

    active_table.switch(new_table);

    active_table
}
