use crate::address::Frame;
use crate::allocator::FrameAllocator;
use crate::paging::mapper::TopLevelP4;
use crate::paging::table::{DirectP4Create, RecurseLevel1, RecurseP4Create, TableSwitch};
use crate::registers::{Cr3, Cr3Flags};
use crate::{EntryFlags, PAGE_SIZE};

pub use self::entry::*;
use self::mapper::Mapper;
use self::table::Table;
use bit_field::BitField;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut, Range};
use core::ptr::Unique;
use sentinel::log;
use table::{AnyLevel, TableLevel4};

mod entry;
pub mod mapper;
pub mod table;
pub mod temporary_page;

const ENTRY_COUNT: u64 = 512;

pub struct ActivePageTable<P4: TableLevel4> {
    p4: Unique<Table<P4>>,
    mapper: Mapper<P4>,
}

impl<P4> ActivePageTable<P4>
where
    P4: TableLevel4<CreateMarker = RecurseP4Create>,
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
    P4: TableLevel4<CreateMarker = DirectP4Create>,
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

pub struct TableManipulationContext<'a, A: FrameAllocator> {
    pub temporary_page: &'a mut temporary_page::TemporaryPage,
    pub allocator: &'a mut A,
}

impl<P4: TopLevelP4> ActivePageTable<P4> {
    fn p4_mut(&mut self) -> &mut Table<P4> {
        // SAFETY: Taking a reference to the page table is valid and safe, in this module
        unsafe { self.p4.as_mut() }
    }

    pub fn with<F, A: FrameAllocator>(
        &mut self,
        table: &mut InactivePageTable<P4>,
        context: &mut TableManipulationContext<A>,
        f: F,
    ) where
        F: FnOnce(&mut Mapper<P4>, &mut A),
    {
        let (level_4_table_frame, _) = Cr3::read();
        let backup = level_4_table_frame;

        {
            // SAFETY: We know that the frame is valid because we're reading it from the cr3
            // which if it's is indeed invalid, this code shouldn't be even executing
            let p4_table = unsafe {
                context
                    .temporary_page
                    .map_table_frame(backup, self, context.allocator)
            };

            self.p4_mut()[511].set(table.p4_frame, EntryFlags::PRESENT | EntryFlags::WRITABLE);
            Cr3::reload();

            f(self, context.allocator);

            p4_table[511].set(backup, EntryFlags::PRESENT | EntryFlags::WRITABLE);
            Cr3::reload();
        }

        // SAFETY: The reference to the page is gone in the scope above
        unsafe { context.temporary_page.unmap(self) };
    }

    /// Switch the page table with the inactive page table
    ///
    /// # Safety
    /// the caller must ensure that by swapping the table does not causes any unsafe
    /// side effects
    pub unsafe fn switch<A: FrameAllocator>(
        &mut self,
        context: &mut TableManipulationContext<A>,
        new_table: InactivePageTable<P4>,
    ) -> InactivePageTable<P4>
    where
        P4::Marker: TableSwitch<P4>,
    {
        <P4::Marker as TableSwitch<P4>>::switch_impl(self, context, new_table)
    }

    /// Switch the page table with the inactive page table, UNCONDITIONALLY
    ///
    /// # Safety
    /// this function is VERY VERY unsafe to use, you must be sure that the P4 isn't split or
    /// paritioned in any way
    pub unsafe fn full_switch(
        &mut self,
        new_table: InactivePageTable<P4>,
    ) -> InactivePageTable<P4> {
        let (level_4_table_frame, _) = Cr3::read();
        let old_table = InactivePageTable::<P4> {
            p4_frame: level_4_table_frame,
            _p4: PhantomData,
        };
        // SAFETY: The inactive page table should be valid if created correctly, and the caller
        // will uphold the contract that there will be no parition of P4
        unsafe {
            Cr3::write(new_table.p4_frame, Cr3Flags::empty());
        }
        old_table
    }

    fn p4(&self) -> &Table<P4> {
        // SAFETY: Taking a reference to the page table is valid and safe, in this module
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

    pub fn create_mappings<F, A>(
        &mut self,
        f: F,
        context: &mut TableManipulationContext<A>,
        options: InactivePageCreateOption,
    ) -> InactivePageTable<P4>
    where
        F: FnOnce(&mut Mapper<P4>, &mut A),
        A: FrameAllocator,
    {
        let mut new_table = InactivePageTable::new(self, context, options);

        self.with(&mut new_table, context, |mapper, allocator| {
            f(mapper, allocator);
        });

        new_table
    }
}

/// InactivePageTable are the table that can be swapped to be an active page table using
/// [`ActivePageTable<T>::switch`]
// FIXME: There will be a minor memory leak if this is dropped.
pub struct InactivePageTable<P4: TopLevelP4> {
    p4_frame: Frame,
    _p4: PhantomData<P4>,
}

/// An argument to the [`InactivePageTable::new`] function, default is empty
#[derive(Debug, Default)]
pub enum InactivePageCreateOption {
    /// Create a new [`InactivePageTable`] with empty p4, (but with recursive mapping)
    #[default]
    Empty,
    /// Create a new [`InactivePageTable`] with the specified range (exclusive) copied from the
    /// active_table to the new [`InactivePageTable`]
    Range(Range<usize>),
    /// Copy all the entries, from the active_table to the new [`InactivePageTable`]
    All,
}

impl InactivePageCreateOption {
    pub fn lower_half() -> Self {
        Self::Range(0..256)
    }
    pub fn upper_half() -> Self {
        Self::Range(256..512)
    }
}

impl<P4: TopLevelP4> InactivePageTable<P4> {
    /// Create a new InactivePage, with a recursive mapping,
    /// specify the copy behavior with the [`InactivePageCreateOption`]
    pub fn new<A: FrameAllocator>(
        active_table: &mut ActivePageTable<P4>,
        context: &mut TableManipulationContext<A>,
        options: InactivePageCreateOption,
    ) -> Self {
        let frame = context.allocator.allocate_frame().expect("no more frames");
        {
            // SAFETY: We know that the frame is valid because it's is being allocated above
            let table = unsafe {
                context
                    .temporary_page
                    .map_table_frame(frame, active_table, context.allocator)
            };
            table.zero();

            match options {
                InactivePageCreateOption::All => {
                    table[0..512].copy_from_slice(&active_table.p4()[0..512]);
                }
                InactivePageCreateOption::Range(range) => {
                    table[range.clone()].copy_from_slice(&active_table.p4()[range]);
                }
                InactivePageCreateOption::Empty => {}
            }

            table[511].set(frame, EntryFlags::PRESENT | EntryFlags::WRITABLE);
        }
        // SAFETY: The reference to the table is gone in the scope
        unsafe { context.temporary_page.unmap(active_table) };

        InactivePageTable::<P4> {
            p4_frame: frame,
            _p4: PhantomData,
        }
    }

    /// Mutate the currently owned p4 table
    ///
    /// # Safety
    /// The caller mustn't mutate the 512th element of the table as it's is the recursive mapping.
    pub unsafe fn table<A: FrameAllocator>(
        &mut self,
        active_table: &mut ActivePageTable<P4>,
        context: &mut TableManipulationContext<A>,
        table_mutate: impl FnOnce(&mut ActivePageTable<P4>, &mut Table<RecurseLevel1>),
    ) {
        {
            // SAFETY: We know that the frame is valid because it's is being allocated in the new
            // function
            let mapped = unsafe {
                context.temporary_page.map_table_frame(
                    self.p4_frame,
                    active_table,
                    context.allocator,
                )
            };
            table_mutate(active_table, mapped);
        }
        // SAFETY: The reference to the table is gone in the scope
        unsafe { context.temporary_page.unmap(active_table) };
    }
}
