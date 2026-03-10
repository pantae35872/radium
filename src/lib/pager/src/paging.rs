use crate::address::{AnyFrame, Frame, Page, PageSize, Size1G, Size2M, Size4K, VirtAddr};
use crate::allocator::FrameAllocator;
use crate::paging::table::entry::Entry;
use crate::paging::table::{
    DirectCreate, RecurseCreate, RootLevel, RootLevelRecurse, RootRecurse, RootRecurseLowerHalf, RootRecurseUpperHalf,
    TableSwitch,
};
use crate::registers::{Cr3, Cr3Flags};
use crate::{EntryFlags, virt_addr_alloc};

use self::mapper::Mapper;
use self::table::Table;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut, Range};
use core::ptr::NonNull;

pub mod mapper;
pub mod table;
pub mod temporary_page;

/// Any implementer of this trait gurentee to transfer it's fields to a new address space (new table)
pub trait Transferable {
    /// Transfer the fields of the [Transferable] implementer to a new address space using the [Transferor]
    ///
    /// The implementation may use the provided [Transferor] to get the new address for the fields
    fn transfer<RefRoot: RootLevel, TargetRoot: RootLevel, A: FrameAllocator>(
        &mut self,
        transferor: &mut Transferor<RefRoot, TargetRoot, A>,
        replace: bool,
    );
}

impl<T> Transferable for Option<T>
where
    T: Transferable,
{
    fn transfer<RefRoot: RootLevel, TargetRoot: RootLevel, A: FrameAllocator>(
        &mut self,
        transferor: &mut Transferor<RefRoot, TargetRoot, A>,
        replace: bool,
    ) {
        if let Some(t) = self.as_mut() {
            t.transfer(transferor, replace);
        }
    }
}

pub struct Transferor<'a, 'b, RefRoot: RootLevel, TargetRoot: RootLevel, A: FrameAllocator> {
    pub(crate) reference_mapping: &'a Mapper<RefRoot>,
    pub(crate) target_mapping: &'b mut Mapper<TargetRoot>,
    pub(crate) allocator: &'b mut A,
}

impl<'a, 'b, RefRoot: RootLevel, TargetRoot: RootLevel, A: FrameAllocator> Transferor<'a, 'b, RefRoot, TargetRoot, A> {
    /// Transfer the original virtual address to the new address space returning the new address in
    /// the process
    ///
    /// # Note
    /// original can be unaligned
    pub fn transfer(&mut self, original: VirtAddr, size: usize, flags: EntryFlags) -> Option<VirtAddr> {
        if size == 0 {
            return None;
        }

        let page_offset = (original.as_u64() & (Size4K::SIZE - 1)) as usize;
        let size_in_pages = (page_offset + size).div_ceil(Size4K::SIZE as usize) as u64;

        let src_start = Page::<Size4K>::from(original);
        let target_start = virt_addr_alloc::<Size4K>(size_in_pages);

        let mut new_mapping =
            Page::<Size4K>::range(src_start, size_in_pages).zip(Page::range(target_start, size_in_pages));

        while let Some((src_page, target_page)) = new_mapping.next() {
            let frame = self.reference_mapping.translate_page(src_page)?;

            match frame {
                AnyFrame::Frame4K(frame) => {
                    unsafe { self.target_mapping.map_to(target_page, frame, flags, self.allocator) };
                }
                // if it is aligned we map it with the huge pages meaning it is at the start of the
                // huge page we map it directly
                huge_frame
                    if src_page.start_address().as_u64() & (huge_frame.size() - 1) == 0
                        && target_page.start_address().as_u64() & (huge_frame.size() - 1) == 0 =>
                {
                    // -1 since this frame is already iterated
                    match new_mapping.advance_by((huge_frame.size() / Size4K::SIZE) as usize - 1) {
                        Ok(_) => {
                            let page = match huge_frame {
                                AnyFrame::Frame2M(..) => {
                                    Page::<Size2M>::containing_address(target_page.start_address()).erase()
                                }
                                AnyFrame::Frame1G(..) => {
                                    Page::<Size1G>::containing_address(target_page.start_address()).erase()
                                }
                                _ => unreachable!("This is matched above"),
                            };
                            unsafe { self.target_mapping.map_to_any(page, huge_frame, flags, self.allocator) };
                        }
                        Err(overrun) => {
                            let start_frame = Frame::<Size4K>::containing_address(huge_frame.start_address());
                            let size = huge_frame.size() / Size4K::SIZE - overrun.get() as u64;
                            for (frame, page) in Frame::range(start_frame, size).zip(Page::range(target_page, size)) {
                                unsafe { self.target_mapping.map_to(page, frame, flags, self.allocator) };
                            }
                        }
                    };
                }
                // if it is not aligned we have to offset the mapping by the misalignment
                huge_frame => {
                    let misalignment = src_page.start_address().as_u64() & (huge_frame.size() - 1);
                    let pages_per_huge = huge_frame.size() / Size4K::SIZE;
                    let offset_in_huge = misalignment / Size4K::SIZE;
                    let remaining_in_huge = pages_per_huge - offset_in_huge;
                    let overrun = match new_mapping.advance_by((remaining_in_huge - 1) as usize) {
                        Ok(_) => 0,
                        Err(over) => over.into(),
                    } as u64;
                    let start_frame = Frame::<Size4K>::containing_address(huge_frame.start_address() + misalignment);
                    let size = remaining_in_huge - overrun;
                    for (frame, page) in Frame::range(start_frame, size).zip(Page::range(target_page, size)) {
                        unsafe { self.target_mapping.map_to(page, frame, flags, self.allocator) };
                    }
                }
            }
        }

        Some(target_start.start_address() + page_offset)
    }
}

const ENTRY_COUNT: u64 = 512;

pub struct ActivePageTable<Root: RootLevel> {
    p4: NonNull<Table<Root>>,
    mapper: Mapper<Root>,
}

impl<Root> ActivePageTable<Root>
where
    Root: RootLevel<CreateMarker = RecurseCreate>,
{
    /// Create a mapper from the currently active recursive mapped page table
    ///
    /// # Safety
    ///
    /// The caller must ensure that there is currently only one instance or access to the
    /// [`ActivePageTable`] entries at a time, there can be multiple [`ActivePageTable`]
    /// pointing to the same set of entries but their must be only one access to a certain entry at a time,
    /// this can be done through a lock.
    pub unsafe fn new() -> ActivePageTable<Root> {
        // SAFETY: we've already tell the require preconditions above
        unsafe { ActivePageTable { p4: Root::CreateMarker::create(), mapper: Mapper::new() } }
    }
}

impl<Root> ActivePageTable<Root>
where
    Root: RootLevel<CreateMarker = DirectCreate>,
{
    /// Create a page table from the provided page table address
    ///
    /// # Safety
    ///
    /// The caller must ensure that the provided p4 is valid,
    /// and is the only mutable reference to the page table
    pub unsafe fn new_custom(p4: *mut Table<Root>) -> ActivePageTable<Root> {
        // SAFETY: we've already tell the require preconditions above
        unsafe { ActivePageTable { p4: Root::CreateMarker::create(p4), mapper: Mapper::new_custom(p4) } }
    }
}

impl<Root: RootLevel> Deref for ActivePageTable<Root> {
    type Target = Mapper<Root>;

    fn deref(&self) -> &Mapper<Root> {
        &self.mapper
    }
}

impl<Root: RootLevel> DerefMut for ActivePageTable<Root> {
    fn deref_mut(&mut self) -> &mut Mapper<Root> {
        &mut self.mapper
    }
}

pub struct TableManipulationContext<'a, A: FrameAllocator> {
    pub temporary_page_mapper: Option<&'a mut ActivePageTable<RootRecurseUpperHalf>>,
    pub temporary_page: &'a mut temporary_page::TemporaryTable,
    pub allocator: &'a mut A,
}

impl<'a, A: FrameAllocator> TableManipulationContext<'a, A> {
    /// Just a helper, See [`temporary_page::TemporaryTable::map_table_frame`] for more info
    ///
    /// # Safety
    /// [`temporary_page::TemporaryTable::map_table_frame`] Safety section
    pub unsafe fn map_temporary_page<'b, Root: RootLevel, MapRoot: RootLevel>(
        &'b mut self,
        frame: Frame<Size4K>,
        active_table: &mut ActivePageTable<Root>,
    ) -> (&'b mut Table<MapRoot>, &'b mut A) {
        (
            unsafe {
                match self.temporary_page_mapper.as_mut() {
                    Some(mapper) => self.temporary_page.map_table_frame(frame, mapper, self.allocator),
                    None => self.temporary_page.map_table_frame(frame, active_table, self.allocator),
                }
            },
            &mut self.allocator,
        )
    }

    /// Just a helper, See [`temporary_page::TemporaryTable::unmap`] for more info
    ///
    /// # Safety
    /// [`temporary_page::TemporaryTable::unmap`] Safety section
    pub unsafe fn unmap_temporary_page<Root: RootLevel>(&mut self, active_table: &mut ActivePageTable<Root>) {
        unsafe {
            match self.temporary_page_mapper.as_mut() {
                Some(mapper) => self.temporary_page.unmap(mapper),
                None => self.temporary_page.unmap(active_table),
            }
        }
    }
}

impl ActivePageTable<RootRecurse> {
    pub fn split(self) -> (ActivePageTable<RootRecurseLowerHalf>, ActivePageTable<RootRecurseUpperHalf>) {
        // SAFETY: This is safe because by our model, there should only be one ActivePageTable at a
        // time, BUT. we're spliting the active page table in 2 halves, so there couldn't be a reference
        // to the same entry in the p4 level. if we're not doing some weird tricks like having the
        // p4 entry on the lower half pointing to the same p3 entry that was pointed by the upper
        // halfs, which we're not.... hopefully
        unsafe { (ActivePageTable::<RootRecurseLowerHalf>::new(), ActivePageTable::<RootRecurseUpperHalf>::new()) }
    }
}

impl ActivePageTable<RootRecurseUpperHalf> {
    /// Switch the page table with the inactive page table, returns the current upper half as
    /// [InactivePageTable] and lower half as the new [ActivePageTable<RootRecurseLowerHalf>]
    ///
    /// # Safety
    /// The caller must ensure that there is no [ActivePageTable<RootRecurseLowerHalf>] object available,
    /// or they must guarantee the mutual exclusivity themselfs
    pub unsafe fn switch_split(
        &mut self,
        new_table: InactivePageTable<RootRecurse>,
    ) -> (InactivePageTable<RootRecurseUpperHalf>, ActivePageTable<RootRecurseLowerHalf>) {
        let (level_4_table_frame, _) = Cr3::read();
        let old_table = InactivePageTable::<RootRecurseUpperHalf> { p4_frame: level_4_table_frame, _p4: PhantomData };
        // SAFETY: The inactive page table is gurentee to be valid, by its safety contracts
        unsafe {
            Cr3::write(new_table.p4_frame, Cr3Flags::empty());
        }

        // SAFETY: The exclusivity contract are uphold by the provided inactive page table,
        // since we're switch a whole table from the provided inactive page table, there is a left
        // over lower half, so we return that.
        (old_table, unsafe { ActivePageTable::<RootRecurseLowerHalf>::new() })
    }
}

impl<Root: RootLevel> ActivePageTable<Root> {
    fn p4_mut(&mut self) -> &mut Table<Root> {
        // SAFETY: Taking a reference to the page table is valid and safe, in this module
        unsafe { self.p4.as_mut() }
    }

    /// Access the mapping of the provided [`InactivePageTable`].
    ///
    /// # Notes
    ///
    /// the currently active page table didn't get swapped out in this process, this just change
    /// the 512th entry of the currently active page table with the address of the
    /// [`InactivePageTable`], The Root bound is restricted to just [`TopLevelRecurse`] because
    /// this requires a recursively mapped [`InactivePageTable`]
    ///
    /// # Safety
    ///
    /// The caller mustn't mutate the [InactivePageTable] in the provided mapper function, to
    /// violate the mutable exclusivity of the entries, See [InactivePageTable] Safety docs
    pub unsafe fn with<F, A: FrameAllocator, InactiveRoot: RootLevelRecurse, R>(
        &mut self,
        table: &mut InactivePageTable<InactiveRoot>,
        context: &mut TableManipulationContext<A>,
        f: F,
    ) -> R
    where
        F: FnOnce(&mut Mapper<InactiveRoot>, &mut A) -> R,
    {
        let (level_4_table_frame, _) = Cr3::read();
        let backup = level_4_table_frame;
        let result;

        {
            // SAFETY: We know that the frame is valid because we're reading it from the cr3
            // which if it's is indeed invalid, this code shouldn't be even executing
            let (p4_table, allocator) = unsafe { context.map_temporary_page::<Root, Root>(backup, self) };

            self.p4_mut()[511].set(table.p4_frame, EntryFlags::PRESENT | EntryFlags::WRITABLE);
            Cr3::reload();

            {
                // SAFETY: This is needed because we can't just pass self onto the mapping
                // function, we have to provide the mapper of the requested type thus we create a
                // new page table of that type just for the mapper, the safety contract is uphold
                // because we're not mutating the entry of the active page table directly,
                // the exclusivity of the inactive page table entries is uphold by the caller
                let custom_table = unsafe { &mut ActivePageTable::<InactiveRoot>::new() };
                result = f(custom_table, allocator);
            }

            p4_table[511].set(backup, EntryFlags::PRESENT | EntryFlags::WRITABLE);
            Cr3::reload();
        }

        // SAFETY: The reference to the page is gone in the scope above
        unsafe { context.unmap_temporary_page(self) };
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
        new_table: InactivePageTable<Root>,
    ) -> InactivePageTable<Root>
    where
        Root::Marker: TableSwitch<Root>,
    {
        <Root::Marker as TableSwitch<Root>>::switch_impl(self, context, new_table)
    }

    /// Switch the page table with the inactive page table, UNCONDITIONALLY
    ///
    /// # Safety
    /// this function is VERY VERY unsafe to use, you must be sure that the root isn't split or
    /// paritioned in any way
    pub unsafe fn full_switch(&mut self, new_table: InactivePageTable<Root>) -> InactivePageTable<Root> {
        let (level_4_table_frame, _) = Cr3::read();
        let old_table = InactivePageTable::<Root> { p4_frame: level_4_table_frame, _p4: PhantomData };
        // SAFETY: The inactive page table should be valid if created correctly, and the caller
        // upholds the contract that there will be no parition of the root
        unsafe {
            Cr3::write(new_table.p4_frame, Cr3Flags::empty());
        }
        old_table
    }

    fn p4(&self) -> &Table<Root> {
        // SAFETY: Taking a reference to the page table is valid and safe, in this module
        unsafe { self.p4.as_ref() }
    }

    /// Copy the entries of one inactive table to another inactive table, you must gurentee mutable
    /// exclusivity of the entries yourself (Read the Safety section)
    ///
    /// # Safety
    /// See [InactivePageTable::new], basically the caller must ensure mutable exclusivity of the
    /// entries themself
    pub unsafe fn copy_mappings_from<A, CopyRoot>(
        &mut self,
        context: &mut TableManipulationContext<A>,
        options: InactivePageCopyOption,
        copy_from: &InactivePageTable<CopyRoot>,
    ) -> InactivePageTable<CopyRoot>
    where
        CopyRoot: RootLevelRecurse,
        A: FrameAllocator,
    {
        let copy_from = copy_from.table(self, context, |_, table| table.entries);
        // SAFETY: The contract is uphold by the caller
        unsafe { InactivePageTable::new_from(self, context, options, &copy_from) }
    }

    /// Create a new mapping, f can be used to map the mapping while the map is being created,
    /// [`InactivePageCopyOption`] can be use to specify the entries that will be copied from the
    /// currently active table, but this must be use with caution (read the safety section)
    ///
    /// # Safety
    /// See [InactivePageTable::new], basically the caller must ensure mutable exclusivity of the
    /// entries themself
    pub unsafe fn create_mappings<F, A, NewRoot: RootLevelRecurse>(
        &mut self,
        f: F,
        context: &mut TableManipulationContext<A>,
        options: InactivePageCopyOption,
    ) -> InactivePageTable<NewRoot>
    where
        F: FnOnce(&mut Mapper<NewRoot>, &mut A),
        A: FrameAllocator,
    {
        // SAFETY: The contract is uphold by the caller
        let mut new_table = unsafe { InactivePageTable::<NewRoot>::new(self, context, options) };

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
pub struct InactivePageTable<Root: RootLevel> {
    p4_frame: Frame<Size4K>,
    _p4: PhantomData<Root>,
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

impl<Root: RootLevel> InactivePageTable<Root> {
    /// Create a new InactivePage, with a recursive mapping,
    /// specify the copy behavior with the [`InactivePageCopyOption`], but that must be done with
    /// caution (Read the Safety section), use the [InactivePageTable::new_from] function if you want to copy from
    /// something else instead of the provided active page table.
    ///
    /// # Safety
    ///
    /// When [`InactivePageCopyOption`] is used but the variant aren't [`InactivePageCopyOption::Empty`]
    /// the caller must gurentee that there will be only one mutable exclusive access to entries within
    /// the new inactive page table while it's active.
    ///
    /// See [`InactivePageTable`] Safety section to see why there might be an exclusivity violation.
    pub unsafe fn new<A: FrameAllocator, ActiveRoot: RootLevel>(
        active_table: &mut ActivePageTable<ActiveRoot>,
        context: &mut TableManipulationContext<A>,
        options: InactivePageCopyOption,
    ) -> Self {
        let frame = context.allocator.allocate_frame().expect("no more frames");
        {
            // SAFETY: We know that the frame is valid because it's is being allocated above
            let (table, ..) = unsafe { context.map_temporary_page(frame, active_table) };
            table.zero();

            match options {
                InactivePageCopyOption::All => {
                    table[0..512].copy_from_slice(&active_table.p4()[0..512]);
                }
                InactivePageCopyOption::Range(range) => {
                    table[range.clone()].copy_from_slice(&active_table.p4()[range]);
                }
                _ => {}
            }

            table[511].set(frame, EntryFlags::PRESENT | EntryFlags::WRITABLE);
        }
        // SAFETY: The reference to the table is gone in the scope
        unsafe { context.unmap_temporary_page(active_table) };

        InactivePageTable::<Root> { p4_frame: frame, _p4: PhantomData }
    }

    /// A variant of [InactivePageTable::new] where you can specify where to copy the entries from
    ///
    /// # Safety
    /// See [InactivePageTable::new] safety section
    pub unsafe fn new_from<A: FrameAllocator, ActiveRoot: RootLevel, FromEntryRoot: RootLevel>(
        active_table: &mut ActivePageTable<ActiveRoot>,
        context: &mut TableManipulationContext<A>,
        options: InactivePageCopyOption,
        copy_from: &[Entry<FromEntryRoot>; ENTRY_COUNT as usize],
    ) -> Self {
        let frame = context.allocator.allocate_frame().expect("no more frames");
        {
            // SAFETY: We know that the frame is valid because it's is being allocated above
            let (table, ..) = unsafe { context.map_temporary_page(frame, active_table) };
            table.zero();

            match options {
                InactivePageCopyOption::All => {
                    table[0..512].copy_from_slice(&copy_from[0..512]);
                }
                InactivePageCopyOption::Range(range) => {
                    table[range.clone()].copy_from_slice(&copy_from[range]);
                }
                _ => {}
            }

            // We must retain recursive mapping
            table[511].set(frame, EntryFlags::PRESENT | EntryFlags::WRITABLE);
        }
        // SAFETY: The reference to the table is gone in the scope
        unsafe { context.unmap_temporary_page(active_table) };

        InactivePageTable::<Root> { p4_frame: frame, _p4: PhantomData }
    }

    /// Reference the currently owned p4 table
    pub fn table<A: FrameAllocator, ActiveRoot: RootLevel, R>(
        &self,
        active_table: &mut ActivePageTable<ActiveRoot>,
        context: &mut TableManipulationContext<A>,
        f: impl FnOnce(&mut ActivePageTable<ActiveRoot>, &Table<Root>) -> R,
    ) -> R {
        let result;
        {
            // SAFETY: We know that the frame is valid because it's is being allocated in the new
            // function
            let (mapped, ..) = unsafe { context.map_temporary_page(self.p4_frame, active_table) };
            result = f(active_table, mapped);
        }
        // SAFETY: The reference to the table is gone in the scope
        unsafe { context.unmap_temporary_page(active_table) };
        result
    }

    /// Mutate the currently owned p4 table
    ///
    /// # Safety
    /// The caller mustn't mutate the 512th element of the table as it's is the recursive mapping.
    /// Also the caller must still maintains the exclusivity contract, See [`InactivePageTable`]
    /// safety docs
    pub unsafe fn table_mut<A: FrameAllocator, ActiveRoot: RootLevel, R>(
        &mut self,
        active_table: &mut ActivePageTable<ActiveRoot>,
        context: &mut TableManipulationContext<A>,
        table_mutate: impl FnOnce(&mut ActivePageTable<ActiveRoot>, &mut Table<Root>) -> R,
    ) -> R {
        let result;
        {
            // SAFETY: We know that the frame is valid because it's is being allocated in the new
            // function
            let (mapped, ..) = unsafe { context.map_temporary_page(self.p4_frame, active_table) };
            result = table_mutate(active_table, mapped);
        }
        // SAFETY: The reference to the table is gone in the scope
        unsafe { context.unmap_temporary_page(active_table) };
        result
    }
}
