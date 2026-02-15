use core::marker::PhantomData;
use core::ops::{Index, IndexMut, Range};
use core::ptr::Unique;

use crate::address::{Size1G, Size2M, Size4K};
use crate::allocator::FrameAllocator;
use crate::paging::mapper::TopLevelP4;
use crate::paging::{ActivePageTable, InactivePageTable, TableManipulationContext};
use crate::registers::tlb;

use super::{ENTRY_COUNT, Entry, EntryFlags};

macro_rules! level {
    ($level: ty[$size: ty], $marker: ty) => {
        impl TableLevel for $level {
            type Marker = RecurseHierarchicalLevelMarker<0, ENTRY_COUNT>;
            type FrameSize = $size;
        }

        impl AnyLevel for Table<$level> {
            fn entries(&self) -> [u64; ENTRY_COUNT as usize] {
                todo!()
            }

            fn next(&self, _index: u64) -> Option<&dyn AnyLevel> {
                None
            }
        }
    };
}

macro_rules! hierarchical_level {
    ($current: ty[$current_size:ty] => $next: ty[$next_size:ty], $marker: ty) => {
        impl HierarchicalLevel for $current
        where
            $next: TableLevel<FrameSize = $next_size>,
        {
            type NextLevel = $next;
        }

        impl TableLevel for $current {
            type Marker = $marker;
            type FrameSize = $current_size;
        }

        impl AnyLevel for Table<$current> {
            fn entries(&self) -> [u64; ENTRY_COUNT as usize] {
                todo!()
                //self.entries
            }

            fn next(&self, index: u64) -> Option<&dyn AnyLevel> {
                self.next_table(index).map(|t| t as &dyn AnyLevel)
            }
        }
    };
}

macro_rules! impl_level_recurse {
    // Base case: nothing more to implement
    ($last:ty[$size:ty]) => {
        level!($last[$size], RecurseHierarchicalLevelMarker<0, ENTRY_COUNT>);
    };

    // Recursive case
    ($current:ty[$current_size:ty] => $next:ty[$next_size:ty] $(=> $rest:ty[$rest_size:ty])*) => {
        hierarchical_level!($current[$current_size] => $next[$next_size], RecurseHierarchicalLevelMarker<0, ENTRY_COUNT>);
        impl_level_recurse!($next[$next_size] $(=> $rest[$rest_size])*);
    };
}

macro_rules! impl_level_direct {
    // Base case: nothing more to implement
    ($last:ty[$size:ty]) => {
        level!($last[$size], DirectHierarchicalLevelMarker<0, ENTRY_COUNT>);
    };

    // Recursive case
    ($current:ty[$current_size:ty] => $next:ty[$next_size:ty] $(=> $rest:ty[$rest_size:ty])*) => {
        hierarchical_level!($current[$current_size] => $next[$next_size], DirectHierarchicalLevelMarker<0, ENTRY_COUNT>);
        impl_level_direct!($next[$next_size] $(=> $rest[$rest_size])*);
    };
}

pub trait NextTableAddress {
    fn next_table_address_impl<L>(table: &Table<L>, index: u64) -> Option<u64>
    where
        L: TableLevel;
}

pub struct Table<L: TableLevel> {
    pub entries: [Entry<L>; ENTRY_COUNT as usize],
    level: PhantomData<L>,
}

impl<L> Table<L>
where
    L: TableLevel,
{
    pub fn zero(&mut self) {
        for entry in self.entries.iter_mut() {
            entry.set_unused();
        }
    }
}

impl<L> Table<L>
where
    L: HierarchicalLevel,
    L::Marker: NextTableAddress,
{
    fn next_table_address(&self, index: u64) -> Option<u64> {
        <L::Marker as NextTableAddress>::next_table_address_impl(self, index)
    }

    pub fn next_table(&self, index: u64) -> Option<&Table<L::NextLevel>> {
        self.next_table_address(index).map(|address| unsafe { &*(address as *const _) })
    }

    pub fn next_table_mut(&mut self, index: u64) -> Option<&mut Table<L::NextLevel>> {
        self.next_table_address(index).map(|address| unsafe { &mut *(address as *mut _) })
    }

    pub fn is_huge_page(&self, index: u64) -> bool {
        self.entries[index as usize].flags().contains(EntryFlags::HUGE_PAGE)
    }

    pub fn next_table_create<A>(
        &mut self,
        index: u64,
        allocator: &mut A,
    ) -> Result<&mut Table<L::NextLevel>, &mut Entry<L>>
    where
        A: FrameAllocator,
    {
        if self.is_huge_page(index) {
            return Err(&mut self.entries[index as usize]);
        }
        if self.next_table(index).is_none() {
            let frame = allocator.allocate_frame().expect("no frames available");
            self.entries[index as usize]
                .set(frame, EntryFlags::PRESENT | EntryFlags::WRITABLE | EntryFlags::USER_ACCESSIBLE);
            self.next_table_mut(index).unwrap().zero();
        }
        Ok(self.next_table_mut(index).unwrap())
    }
}

impl<L> Index<Range<usize>> for Table<L>
where
    L: TableLevel,
{
    type Output = [Entry<L>];

    fn index(&self, range: Range<usize>) -> &Self::Output {
        &self.entries[range]
    }
}

impl<L> IndexMut<Range<usize>> for Table<L>
where
    L: TableLevel,
{
    fn index_mut(&mut self, range: Range<usize>) -> &mut Self::Output {
        &mut self.entries[range]
    }
}

impl<L> Index<usize> for Table<L>
where
    L: TableLevel,
{
    type Output = Entry<L>;

    fn index(&self, index: usize) -> &Entry<L> {
        &self.entries[index]
    }
}

impl<L> IndexMut<usize> for Table<L>
where
    L: TableLevel,
{
    fn index_mut(&mut self, index: usize) -> &mut Entry<L> {
        &mut self.entries[index]
    }
}

pub trait TableLevel {
    type Marker: NextTableAddress;
    type FrameSize;
}

pub trait TableLevel4: TableLevel
where
    Self: Sized,
{
    type CreateMarker;
}

pub trait HierarchicalLevel: TableLevel {
    type NextLevel: TableLevel;
}

pub trait AnyLevel {
    fn entries(&self) -> [u64; ENTRY_COUNT as usize];

    fn next(&self, index: u64) -> Option<&dyn AnyLevel>;
}

pub trait TableSwitch<P4: TopLevelP4> {
    fn switch_impl<A: FrameAllocator>(
        active_page_table: &mut ActivePageTable<P4>,
        context: &mut TableManipulationContext<A>,
        new_table: InactivePageTable<P4>,
    ) -> InactivePageTable<P4>;
}

pub struct RecurseHierarchicalLevelMarker<const START: u64, const END: u64>;

impl<const START: u64, const END: u64, P4> TableSwitch<P4> for RecurseHierarchicalLevelMarker<START, END>
where
    P4: TopLevelP4<Marker = RecurseHierarchicalLevelMarker<START, END>>,
{
    fn switch_impl<A: FrameAllocator>(
        active_page_table: &mut ActivePageTable<P4>,
        context: &mut TableManipulationContext<A>,
        mut new_table: InactivePageTable<P4>,
    ) -> InactivePageTable<P4> {
        if START == 0 && END == ENTRY_COUNT {
            // SAFETY: The contract is checked above and the impl where clauses guarantee that P4
            // is the top level
            return unsafe { active_page_table.full_switch(new_table) };
        }

        // SAFETY: We're swapping out the now inactive ranges of START..END the exclusivity contract is uphold
        // by the previous [`ActivePageTable::switch`] call
        let old_table = unsafe {
            InactivePageTable::new(
                active_page_table,
                context,
                super::InactivePageCopyOption::Range(START as usize..END as usize),
                None,
            )
        };

        // SAFETY: We did mutate 512th element if the END is 512, but the impl where clauses guarantee that P4 is a recursive mapped.
        // and the InactivePageTable is always recursively mapped.
        unsafe {
            new_table.table_mut(active_page_table, context, |active_table, table| {
                active_table.p4_mut()[START as usize..END as usize]
                    .copy_from_slice(&table[START as usize..END as usize])
            })
        };

        tlb::full_flush();

        old_table
    }
}

impl<const START: u64, const END: u64> NextTableAddress for RecurseHierarchicalLevelMarker<START, END> {
    fn next_table_address_impl<L>(table: &Table<L>, index: u64) -> Option<u64>
    where
        L: TableLevel,
    {
        assert!(index >= START && index < END, "Page table index out of the accessable bounds");
        let entry_flags = table[index as usize].flags();
        if entry_flags.contains(EntryFlags::PRESENT) && !entry_flags.contains(EntryFlags::HUGE_PAGE) {
            let table_address = table as *const _ as u64;
            Some((table_address << 9) | (index << 12))
        } else {
            None
        }
    }
}

pub struct RecurseP4Create;

impl RecurseP4Create {
    /// Create a new recursive p4 table pointer
    ///
    /// # Safety
    ///
    /// the caller must ensure that the current active table is recursive mapped
    pub unsafe fn create<T: TableLevel4>() -> Unique<Table<T>> {
        unsafe { Unique::new_unchecked(0xffffffff_fffff000 as *mut _) }
    }
}

pub enum RecurseLevel4 {}

impl TableLevel4 for RecurseLevel4 {
    type CreateMarker = RecurseP4Create;
}

hierarchical_level!(RecurseLevel4[()] => RecurseLevel3[Size1G], RecurseHierarchicalLevelMarker<0, ENTRY_COUNT>);

pub enum RecurseLevel4LowerHalf {}

impl TableLevel4 for RecurseLevel4LowerHalf {
    type CreateMarker = RecurseP4Create;
}

hierarchical_level!(RecurseLevel4LowerHalf[()] => RecurseLevel3[Size1G], RecurseHierarchicalLevelMarker<0, 256>);

pub enum RecurseLevel4UpperHalf {}

impl TableLevel4 for RecurseLevel4UpperHalf {
    type CreateMarker = RecurseP4Create;
}

hierarchical_level!(RecurseLevel4UpperHalf[()] => RecurseLevel3[Size1G], RecurseHierarchicalLevelMarker<256, { ENTRY_COUNT - 1 }>);

pub enum RecurseLevel3 {}
pub enum RecurseLevel2 {}
pub enum RecurseLevel1 {}

impl_level_recurse!(RecurseLevel3[Size1G] => RecurseLevel2[Size2M] => RecurseLevel1[Size4K]);

pub struct DirectHierarchicalLevelMarker<const START: u64, const END: u64>;

impl<const START: u64, const END: u64> NextTableAddress for DirectHierarchicalLevelMarker<START, END> {
    fn next_table_address_impl<L>(table: &Table<L>, index: u64) -> Option<u64>
    where
        L: TableLevel,
    {
        assert!(index >= START && index < END, "Page table index out of the accessable bounds");
        let entry_flags = table[index as usize].flags();
        if entry_flags.contains(EntryFlags::PRESENT) && !entry_flags.contains(EntryFlags::HUGE_PAGE) {
            Some(table[index as usize].value & 0x000fffff_fffff000)
        } else {
            None
        }
    }
}

pub struct DirectP4Create;

impl DirectP4Create {
    /// Create a new p4 table from the provided table pointer
    ///
    /// # Safety
    ///
    /// the caller must ensure that the table pointer is valid and mapped
    pub unsafe fn create<T: TableLevel4>(p4: *mut Table<T>) -> Unique<Table<T>> {
        unsafe { Unique::new_unchecked(p4) }
    }
}

pub enum DirectLevel4 {}

impl TableLevel4 for DirectLevel4 {
    type CreateMarker = DirectP4Create;
}

pub enum DirectLevel3 {}
pub enum DirectLevel2 {}
pub enum DirectLevel1 {}

impl_level_direct!(DirectLevel4[()] => DirectLevel3[Size1G] => DirectLevel2[Size2M] => DirectLevel1[Size4K]);
