use crate::address::Frame;
use crate::allocator::FrameAllocator;
use crate::paging::mapper::{TopLevelP4, TopLevelRecurse};
use crate::paging::table::{
    DirectP4Create, RecurseLevel1, RecurseLevel4, RecurseLevel4LowerHalf, RecurseLevel4UpperHalf,
    RecurseP4Create, TableSwitch,
};
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
    /// The caller must ensure that there is currently only one instance or access to the
    /// [`ActivePageTable`] entries at a time, there can be multiple [`ActivePageTable`]
    /// pointing to the same set of entries but their must be only one access to a certain entry at a time,
    /// this can be done through a lock.
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

impl ActivePageTable<RecurseLevel4> {
    pub fn split(
        self,
    ) -> (
        ActivePageTable<RecurseLevel4LowerHalf>,
        ActivePageTable<RecurseLevel4UpperHalf>,
    ) {
        // SAFETY: This is safe because by our model, there should only be one ActivePageTable at a
        // time, BUT. we're spliting the active page table in 2 halves, so there couldn't be a reference
        // to the same entry in the p4 level. if we're not doing some weird tricks like having the
        // p4 entry on the lower half pointing to the same p3 entry that was pointed by the upper
        // halfs, which we're not.... hopefully
        unsafe {
            (
                ActivePageTable::<RecurseLevel4LowerHalf>::new(),
                ActivePageTable::<RecurseLevel4UpperHalf>::new(),
            )
        }
    }
}

impl ActivePageTable<RecurseLevel4UpperHalf> {
    /// Switch the page table with the inactive page table
    ///
    /// # Safety
    /// The caller must ensure that swapping the page table doesn't cause unsafe
    /// any side effects
    pub unsafe fn switch_split(
        &mut self,
        new_table: InactivePageTable<RecurseLevel4>,
    ) -> (
        InactivePageTable<RecurseLevel4UpperHalf>,
        ActivePageTable<RecurseLevel4LowerHalf>,
    ) {
        let (level_4_table_frame, _) = Cr3::read();
        let old_table = InactivePageTable::<RecurseLevel4UpperHalf> {
            p4_frame: level_4_table_frame,
            _p4: PhantomData,
        };
        // SAFETY: The inactive page table is gurentee to be valid, by its safety contracts
        unsafe {
            Cr3::write(new_table.p4_frame, Cr3Flags::empty());
        }

        // SAFETY: The exclusivity contract are uphold by the provided inactive page table,
        // since we're switch a whole table from the provided inactive page table, there is a left
        // over lower half, so we return that.
        (old_table, unsafe {
            ActivePageTable::<RecurseLevel4LowerHalf>::new()
        })
    }
}

impl<P4: TopLevelP4> ActivePageTable<P4> {
    fn p4_mut(&mut self) -> &mut Table<P4> {
        // SAFETY: Taking a reference to the page table is valid and safe, in this module
        unsafe { self.p4.as_mut() }
    }

    /// Access the mapping of the provided [`InactivePageTable`].
    ///
    /// # Notes
    ///
    /// the currently active page table didn't get swapped out in this process, this just change
    /// the 512th entry of the currently active page table with the address of the
    /// [`InactivePageTable`], The P4 bound is restricted to just [`TopLevelRecurse`] because
    /// this requires a recursively mapped [`InactivePageTable`]
    ///
    /// # Safety
    ///
    /// The caller mustn't mutate the [InactivePageTable] in the provided mapper function, to
    /// violate the mutable exclusivity of the entries, See [InactivePageTable] Safety docs
    pub unsafe fn with<F, A: FrameAllocator, R, RecurseP4: TopLevelRecurse>(
        &mut self,
        table: &mut InactivePageTable<RecurseP4>,
        context: &mut TableManipulationContext<A>,
        f: F,
    ) -> R
    where
        F: FnOnce(&mut Mapper<RecurseP4>, &mut A) -> R,
    {
        let (level_4_table_frame, _) = Cr3::read();
        let backup = level_4_table_frame;
        let result;

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

            {
                // SAFETY: This is needed because we can't just pass self onto the mapping
                // function, we have to provide the mapper of the requested type thus we create a
                // new page table of that type just for the mapper, the safety contract is uphold
                // because we're not mutating the entry of the active page table directly,
                // the exclusivity of the inactive page table entries is uphold by the caller
                let custom_table = unsafe { &mut ActivePageTable::<RecurseP4>::new() };
                result = f(custom_table, context.allocator);
            }

            p4_table[511].set(backup, EntryFlags::PRESENT | EntryFlags::WRITABLE);
            Cr3::reload();
        }

        // SAFETY: The reference to the page is gone in the scope above
        unsafe { context.temporary_page.unmap(self) };
        result
    }

    /// Switch the page table with the inactive page table
    ///
    /// # Safety
    /// the caller must ensure that by swapping the table doesn't break the exclusivity of the
    /// p3 entries or any entries.
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
        // upholds the contract that there will be no parition of P4
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

    /// Create a new mapping, f can be used to map the mapping while the map is being created,
    /// [`InactivePageCopyOption`] can be use to specify the entries that will be copied from the
    /// currently active table, but this must be use with caution (read the safety section)
    ///
    /// # Safety
    /// See [InactivePageTable::new], basically the caller must ensure mutable exclusivity of the
    /// entries themself
    pub unsafe fn create_mappings<F, A, RecurseP4: TopLevelRecurse>(
        &mut self,
        f: F,
        context: &mut TableManipulationContext<A>,
        options: InactivePageCopyOption,
    ) -> InactivePageTable<RecurseP4>
    where
        F: FnOnce(&mut Mapper<RecurseP4>, &mut A),
        A: FrameAllocator,
    {
        // SAFETY: The contract is uphold by the caller
        let mut new_table = unsafe { InactivePageTable::<RecurseP4>::new(self, context, options) };

        // SAFETY: The contract is uphold by the caller
        unsafe {
            self.with(&mut new_table, context, |mapper, allocator| {
                f(mapper, allocator);
            })
        };

        new_table
    }
}

/// InactivePageTable are the table that can be swapped to be an active page table using
/// [`ActivePageTable<T>::switch`],
///
/// # Safety
/// Most of the function in this struct is unsafe, because the user must uphold the exclusivity of the
/// entries themself.
///
/// There can be an exclusivity violation because there can be multiple [`ActivePageTable`] on
/// different cores, for example if someone used this function multiple times to create new
/// [`InactivePageTable`] with the same entries, and then they send it across different cores,
/// when each core swapped out the [`ActivePageTable`] with these copied [`InactivePageTable`],
/// without a proper lock in place, there will be a multiple mutable reference
/// to the same p3 entries of the [`ActivePageTable`].
// FIXME: There will be a minor memory leak if this is dropped.
pub struct InactivePageTable<P4: TopLevelP4> {
    p4_frame: Frame,
    _p4: PhantomData<P4>,
}

/// An argument to the [`InactivePageTable::new`] function, default is empty
#[derive(Debug, Default)]
pub enum InactivePageCopyOption {
    /// Create a new [`InactivePageTable`] with empty p4, (but with recursive mapping)
    #[default]
    Empty,
    /// Create a new [`InactivePageTable`] with the specified range (exclusive) copied from the
    /// active_table to the new [`InactivePageTable`]
    Range(Range<usize>),
    /// Copy all the entries, from the active_table to the new [`InactivePageTable`]
    All,
}

impl InactivePageCopyOption {
    pub fn lower_half() -> Self {
        Self::Range(0..256)
    }
    pub fn upper_half() -> Self {
        Self::Range(256..512)
    }
}

impl<P4: TopLevelP4> InactivePageTable<P4> {
    /// Create a new InactivePage, with a recursive mapping,
    /// specify the copy behavior with the [`InactivePageCreateOption`], but that must be done with
    /// caution (Read the Safety section)
    ///
    /// # Safety
    ///
    /// When [`InactivePageCopyOption`] is used but the variant aren't [`InactivePageCopyOption::Empty`]
    /// the caller must gurentee that there will be only one mutable exclusive access to entries within
    /// the new inactive page table while it's active.
    ///
    /// See [`InactivePageTable`] Safety section to see why there might be an exclusivity violation.
    pub unsafe fn new<A: FrameAllocator, ActiveP4: TopLevelP4>(
        active_table: &mut ActivePageTable<ActiveP4>,
        context: &mut TableManipulationContext<A>,
        options: InactivePageCopyOption,
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
                InactivePageCopyOption::All => {
                    table[0..512].copy_from_slice(&active_table.p4()[0..512]);
                }
                InactivePageCopyOption::Range(range) => {
                    table[range.clone()].copy_from_slice(&active_table.p4()[range]);
                }
                InactivePageCopyOption::Empty => {}
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
    /// Also the caller must still maintains the exclusivity contract, See [`InactivePageTable`]
    /// safety docs
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
