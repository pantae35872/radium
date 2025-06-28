use crate::address::{Frame, Page};
use crate::allocator::FrameAllocator;
use crate::paging::mapper::TopLevelP4;
use crate::registers::{Cr3, Cr3Flags};
use crate::{EntryFlags, PAGE_SIZE};

pub use self::entry::*;
use self::mapper::Mapper;
use self::table::Table;
use self::temporary_page::TemporaryPage;
use bit_field::BitField;
use core::ops::{Deref, DerefMut};
use core::ptr::Unique;
use sentinel::log;
use table::{AnyLevel, DirectP4Create, RecurseP4Create, TableLevel4};

mod entry;
pub mod mapper;
pub mod table;
mod temporary_page;

const ENTRY_COUNT: u64 = 512;

pub struct ActivePageTable<P4: TableLevel4> {
    p4: Unique<Table<P4>>,
    mapper: Mapper<P4>,
}

impl<P4> ActivePageTable<P4>
where
    P4: TableLevel4,
    P4::CreateMarker: RecurseP4Create<P4>,
{
    /// Create a mapper from the currently active recursive mapped page table
    ///
    /// # Safety
    ///
    /// The caller must ensure that the current active page table is recursive mapped
    pub unsafe fn new() -> ActivePageTable<P4> {
        // SAFETY: we've already tell the require preconditions above
        unsafe {
            ActivePageTable {
                p4: P4::CreateMarker::create(),
                mapper: Mapper::new(),
            }
        }
    }
}

impl<P4> ActivePageTable<P4>
where
    P4: TableLevel4,
    P4::CreateMarker: DirectP4Create<P4>,
{
    /// Create a page table from the provided page table address
    ///
    /// # Safety
    ///
    /// The caller must ensure that the provided p4 is valid,
    /// and is the only mutable reference to the page table
    pub unsafe fn new_custom(p4: *mut Table<P4>) -> ActivePageTable<P4> {
        // SAFETY: we've already tell the require preconditions above
        unsafe {
            ActivePageTable {
                p4: P4::CreateMarker::create(p4),
                mapper: Mapper::new_custom(p4),
            }
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

impl<P4: TopLevelP4> ActivePageTable<P4> {
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
        let (level_4_table_frame, _) = Cr3::read();
        let backup = level_4_table_frame;

        // SAFETY: We know that the frame is valid because we're reading it from the cr3
        // which if it's is indeed invalid, this code shoulnt be even executing
        let p4_table = unsafe { temporary_page.map_table_frame(backup, self) };

        self.p4_mut()[511].set(table.p4_frame, EntryFlags::PRESENT | EntryFlags::WRITABLE);
        Cr3::reload();

        f(self);

        p4_table[511].set(backup, EntryFlags::PRESENT | EntryFlags::WRITABLE);
        Cr3::reload();

        temporary_page.unmap(self);
    }

    /// Switch the page table with the inactive page table
    ///
    /// # Safety
    /// the caller must ensure that by swapping the table does not causes any unsafe
    /// side effects
    pub unsafe fn switch(&mut self, new_table: InactivePageTable) -> InactivePageTable {
        let (level_4_table_frame, _) = Cr3::read();
        let old_table = InactivePageTable {
            p4_frame: level_4_table_frame,
        };
        // SAFETY: The inactive page table should be valid if created correctly
        unsafe {
            Cr3::write(new_table.p4_frame, Cr3Flags::empty());
        }
        old_table
    }

    fn p4(&self) -> &Table<P4> {
        unsafe { self.p4.as_ref() }
    }

    pub fn log_all(&self)
    where
        Table<P4>: AnyLevel,
    {
        log!(Debug, "Current mappings: ");
        log_dyn_recursive(self.p4(), 4, 0);

        fn log_dyn_recursive(table: &dyn AnyLevel, level: u8, base_addr: u64) {
            if level == 1 {
                let (mut last_phys, mut last_virt, mut last_flags) =
                    (0u64, 0u64, EntryFlags::empty());
                let mut is_first = true;
                for (index, entry) in table
                    .entries()
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.flags().contains(EntryFlags::PRESENT))
                {
                    let virt = base_addr as usize | (index << 12);
                    if (last_phys + PAGE_SIZE, last_virt + PAGE_SIZE, last_flags)
                        == (entry.mask_flags(), virt as u64, entry.flags())
                    {
                        if is_first
                            && (last_phys, last_virt, last_flags)
                                != (0u64, 0u64, EntryFlags::empty())
                        {
                            if last_virt.get_bit(47) {
                                last_virt |= 0xffff << 48;
                            }
                            log!(Debug, "-----------------------------------------");
                            log!(
                                Debug,
                                "VIRT: {last_virt:#x} -> PHYS: {last_phys:#x}, {last_flags}",
                            );
                            log!(Debug, "..........................................");
                            is_first = false;
                        }

                        (last_phys, last_virt, last_flags) =
                            (entry.mask_flags(), virt as u64, entry.flags());
                        continue;
                    } else if (last_phys, last_virt, last_flags)
                        != (0u64, 0u64, EntryFlags::empty())
                    {
                        if last_virt.get_bit(47) {
                            last_virt |= 0xffff << 48;
                        }
                        log!(
                            Debug,
                            "VIRT: {last_virt:#x} -> PHYS: {last_phys:#x}, {last_flags}",
                        );
                        log!(Debug, "-----------------------------------------");
                        is_first = true;
                    }
                    (last_phys, last_virt, last_flags) =
                        (entry.mask_flags(), virt as u64, entry.flags());
                }
            }

            for (index, table) in
                (0..ENTRY_COUNT).filter_map(|entry| table.next(entry).map(|table| (entry, table)))
            {
                log_dyn_recursive(
                    table,
                    level - 1,
                    base_addr | (index << ((level - 1) * 9 + 12)),
                );
            }
        }
    }
}

/// InactivePageTable are the table that can be swapped to be an active page table using
/// [`ActivePageTable<T>::switch`]
pub struct InactivePageTable {
    p4_frame: Frame,
}

impl InactivePageTable {
    /// Create a new InactivePage
    pub fn new<P4: TopLevelP4, A: FrameAllocator>(
        allocator: &mut A,
        active_table: &mut ActivePageTable<P4>,
        temporary_page: &mut TemporaryPage,
    ) -> InactivePageTable {
        let frame = allocator.allocate_frame().expect("no more frames");
        {
            // SAFETY: We know that the frame is valid because it's is being allocated above
            let table = unsafe { temporary_page.map_table_frame(frame, active_table) };
            table.zero();
            table[511].set(frame, EntryFlags::PRESENT | EntryFlags::WRITABLE);
        }
        temporary_page.unmap(active_table);

        InactivePageTable { p4_frame: frame }
    }

    pub unsafe fn from_raw_frame(frame: Frame) -> InactivePageTable {
        InactivePageTable { p4_frame: frame }
    }
}

pub fn create_mappings<F, A, P: TopLevelP4>(
    f: F,
    allocator: &mut A,
    active_table: &mut ActivePageTable<P>,
) -> InactivePageTable
where
    F: FnOnce(&mut Mapper<P>, &mut A),
    A: FrameAllocator,
{
    let mut temporary_page = TemporaryPage::new(Page::deadbeef(), allocator);

    let mut new_table = InactivePageTable::new(allocator, active_table, &mut temporary_page);

    active_table.with(&mut new_table, &mut temporary_page, |mapper| {
        f(mapper, allocator);
    });

    new_table
}
