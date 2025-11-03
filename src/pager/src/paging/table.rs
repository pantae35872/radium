use core::marker::PhantomData;
use core::ops::{Index, IndexMut};
use core::ptr::Unique;

use crate::allocator::FrameAllocator;

use super::{ENTRY_COUNT, Entry, EntryFlags};

macro_rules! level {
    ($level: ty, $marker: ty) => {
        impl TableLevel for $level {
            type Marker = RecurseHierarchicalLevelMarker<0, ENTRY_COUNT>;
        }

        impl AnyLevel for Table<$level> {
            fn entries(&self) -> [Entry; ENTRY_COUNT as usize] {
                self.entries
            }

            fn next(&self, _index: u64) -> Option<&dyn AnyLevel> {
                None
            }
        }
    };
}

macro_rules! hierarchical_level {
    ($current: ty, $next: ty, $marker: ty) => {
        impl HierarchicalLevel for $current {
            type NextLevel = $next;
        }

        impl TableLevel for $current {
            type Marker = $marker;
        }

        impl AnyLevel for Table<$current> {
            fn entries(&self) -> [Entry; ENTRY_COUNT as usize] {
                self.entries
            }

            fn next(&self, index: u64) -> Option<&dyn AnyLevel> {
                self.next_table(index).map(|t| t as &dyn AnyLevel)
            }
        }
    };
}

macro_rules! impl_level_recurse {
    // Base case: nothing more to implement
    ($last:ty) => {
        level!($last, RecurseHierarchicalLevelMarker<0, ENTRY_COUNT>);
    };

    // Recursive case
    ($current:ty => $next:ty $(=> $rest:ty)*) => {
        hierarchical_level!($current, $next, RecurseHierarchicalLevelMarker<0, ENTRY_COUNT>);
        impl_level_recurse!($next $(=> $rest)*);
    };
}

macro_rules! impl_level_direct {
    // Base case: nothing more to implement
    ($last:ty) => {
        level!($last, DirectHierarchicalLevelMarker<0, ENTRY_COUNT>);
    };

    // Recursive case
    ($current:ty => $next:ty $(=> $rest:ty)*) => {
        hierarchical_level!($current, $next, DirectHierarchicalLevelMarker<0, ENTRY_COUNT>);
        impl_level_direct!($next $(=> $rest)*);
    };
}

pub trait NextTableAddress {
    fn next_table_address_impl<L>(table: &Table<L>, index: u64) -> Option<u64>
    where
        L: TableLevel;
}

pub struct Table<L: TableLevel> {
    pub entries: [Entry; ENTRY_COUNT as usize],
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
        self.next_table_address(index)
            .map(|address| unsafe { &*(address as *const _) })
    }

    pub fn next_table_mut(&mut self, index: u64) -> Option<&mut Table<L::NextLevel>> {
        self.next_table_address(index)
            .map(|address| unsafe { &mut *(address as *mut _) })
    }

    pub fn next_table_create<A>(
        &mut self,
        index: u64,
        allocator: &mut A,
    ) -> &mut Table<L::NextLevel>
    where
        A: FrameAllocator,
    {
        if self.next_table(index).is_none() {
            assert!(
                !self.entries[index as usize]
                    .flags()
                    .contains(EntryFlags::HUGE_PAGE),
                "mapping code does not support huge pages"
            );
            let frame = allocator.allocate_frame().expect("no frames available");
            self.entries[index as usize].set(frame, EntryFlags::PRESENT | EntryFlags::WRITABLE);
            self.next_table_mut(index).unwrap().zero();
        }
        self.next_table_mut(index).unwrap()
    }
}

impl<L> Index<usize> for Table<L>
where
    L: TableLevel,
{
    type Output = Entry;

    fn index(&self, index: usize) -> &Entry {
        &self.entries[index]
    }
}

impl<L> IndexMut<usize> for Table<L>
where
    L: TableLevel,
{
    fn index_mut(&mut self, index: usize) -> &mut Entry {
        &mut self.entries[index]
    }
}

pub trait TableLevel {
    type Marker: NextTableAddress;
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
    fn entries(&self) -> [Entry; ENTRY_COUNT as usize];

    fn next(&self, index: u64) -> Option<&dyn AnyLevel>;
}

pub struct RecurseHierarchicalLevelMarker<const START: u64, const END: u64>;

impl<const START: u64, const END: u64> NextTableAddress
    for RecurseHierarchicalLevelMarker<START, END>
{
    fn next_table_address_impl<L>(table: &Table<L>, index: u64) -> Option<u64>
    where
        L: TableLevel,
    {
        assert!(
            index >= START && index < END,
            "Page table index out of the accessable bounds"
        );
        let entry_flags = table[index as usize].flags();
        if entry_flags.contains(EntryFlags::PRESENT) && !entry_flags.contains(EntryFlags::HUGE_PAGE)
        {
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

hierarchical_level!(RecurseLevel4, RecurseLevel3, RecurseHierarchicalLevelMarker<0, ENTRY_COUNT>);

pub enum RecurseLevel4LowerHalf {}

impl TableLevel4 for RecurseLevel4LowerHalf {
    type CreateMarker = RecurseP4Create;
}

hierarchical_level!(RecurseLevel4LowerHalf, RecurseLevel3, RecurseHierarchicalLevelMarker<0, 256>);

pub enum RecurseLevel4UpperHalf {}

impl TableLevel4 for RecurseLevel4UpperHalf {
    type CreateMarker = RecurseP4Create;
}

hierarchical_level!(RecurseLevel4UpperHalf, RecurseLevel3, RecurseHierarchicalLevelMarker<256, ENTRY_COUNT>);

pub enum RecurseLevel3 {}
pub enum RecurseLevel2 {}
pub enum RecurseLevel1 {}

impl_level_recurse!(RecurseLevel3 => RecurseLevel2 => RecurseLevel1);

pub struct DirectHierarchicalLevelMarker<const START: u64, const END: u64>;

impl<const START: u64, const END: u64> NextTableAddress
    for DirectHierarchicalLevelMarker<START, END>
{
    fn next_table_address_impl<L>(table: &Table<L>, index: u64) -> Option<u64>
    where
        L: TableLevel,
    {
        assert!(
            index >= START && index < END,
            "Page table index out of the accessable bounds"
        );
        let entry_flags = table[index as usize].flags();
        if entry_flags.contains(EntryFlags::PRESENT) && !entry_flags.contains(EntryFlags::HUGE_PAGE)
        {
            Some(table[index as usize].0 & 0x000fffff_fffff000)
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

impl_level_direct!(DirectLevel4 => DirectLevel3 => DirectLevel2 => DirectLevel1);
