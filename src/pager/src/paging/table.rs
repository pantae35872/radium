use core::marker::PhantomData;
use core::ops::{Index, IndexMut};
use core::ptr::Unique;

use crate::allocator::FrameAllocator;

use super::{Entry, EntryFlags, ENTRY_COUNT};

macro_rules! impl_level_recurse {
    // Base case: nothing more to implement
    ($last:ty) => {
        impl TableLevel for $last {
            type Marker = RecurseHierarchicalLevelMarker;
        }

        impl AnyLevel for Table<$last> {
            fn entries(&self) -> [Entry; ENTRY_COUNT as usize] {
                self.entries
            }
            fn next(&self, _index: u64) -> Option<&dyn AnyLevel> {
                None
            }
        }
    };

    // Recursive case
    ($current:ty => $next:ty $(=> $rest:ty)*) => {
        impl HierarchicalLevel for $current {
            type NextLevel = $next;
        }

        impl TableLevel for $current {
            type Marker = RecurseHierarchicalLevelMarker;
        }


        impl AnyLevel for Table<$current> {
            fn entries(&self) -> [Entry; ENTRY_COUNT as usize] {
                self.entries
            }
            fn next(&self, index: u64) -> Option<&dyn AnyLevel> {
                self.next_table(index).map(|t| t as &dyn AnyLevel)
            }
        }

        impl_level_recurse!($next $(=> $rest)*);
    };
}

macro_rules! impl_level_direct {
    // Base case: nothing more to implement
    ($last:ty) => {
        impl TableLevel for $last {
            type Marker = DirectHierarchicalLevelMarker;
        }

        impl AnyLevel for Table<$last> {
            fn entries(&self) -> [Entry; ENTRY_COUNT as usize] {
                self.entries
            }
            fn next(&self, _index: u64) -> Option<&dyn AnyLevel> {
                None
            }
        }
    };

    // Recursive case
    ($current:ty => $next:ty $(=> $rest:ty)*) => {
        impl HierarchicalLevel for $current {
            type NextLevel = $next;
        }

        impl TableLevel for $current {
            type Marker = DirectHierarchicalLevelMarker;
        }

        impl AnyLevel for Table<$current> {
            fn entries(&self) -> [Entry; ENTRY_COUNT as usize] {
                self.entries
            }
            fn next(&self, index: u64) -> Option<&dyn AnyLevel> {
                self.next_table(index).map(|t| t as &dyn AnyLevel)
            }
        }

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

impl NextTableAddress for RecurseHierarchicalLevelMarker {
    fn next_table_address_impl<L>(table: &Table<L>, index: u64) -> Option<u64>
    where
        L: TableLevel,
    {
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

impl NextTableAddress for DirectHierarchicalLevelMarker {
    fn next_table_address_impl<L>(table: &Table<L>, index: u64) -> Option<u64>
    where
        L: TableLevel,
    {
        let entry_flags = table[index as usize].flags();
        if entry_flags.contains(EntryFlags::PRESENT) && !entry_flags.contains(EntryFlags::HUGE_PAGE)
        {
            Some(table[index as usize].0 & 0x000fffff_fffff000)
        } else {
            None
        }
    }
}

impl DirectP4Marker {}

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

pub struct DirectHierarchicalLevelMarker;
pub struct RecurseHierarchicalLevelMarker;
pub struct RecurseP4Marker;
pub struct DirectP4Marker;
pub trait RecurseP4Create<T>
where
    T: TableLevel,
{
    /// Create a new recursive p4 table pointer
    ///
    /// # Safety
    ///
    /// the caller must ensure that the current active table is recursive mapped
    unsafe fn create() -> Unique<Table<T>> {
        unsafe { Unique::new_unchecked(0xffffffff_fffff000 as *mut _) }
    }
}
pub trait DirectP4Create<T>
where
    T: TableLevel,
{
    /// Create a new p4 table from the provided table pointer
    ///
    /// # Safety
    ///
    /// the caller must ensure that the table pointer is valid and mapped
    unsafe fn create(p4: *mut Table<T>) -> Unique<Table<T>> {
        unsafe { Unique::new_unchecked(p4) }
    }
}

impl<T> RecurseP4Create<T> for RecurseP4Marker where T: TableLevel4 {}
impl<T> DirectP4Create<T> for DirectP4Marker where T: TableLevel4 {}

pub enum RecurseLevel4 {}
pub enum RecurseLevel3 {}
pub enum RecurseLevel2 {}
pub enum RecurseLevel1 {}

pub enum DirectLevel4 {}
pub enum DirectLevel3 {}
pub enum DirectLevel2 {}
pub enum DirectLevel1 {}

pub trait TableLevel {
    type Marker: NextTableAddress;
}

pub trait TableLevel4: TableLevel
where
    Self: Sized,
{
    type CreateMarker;
}

impl TableLevel4 for RecurseLevel4 {
    type CreateMarker = RecurseP4Marker;
}

impl TableLevel4 for DirectLevel4 {
    type CreateMarker = DirectP4Marker;
}

pub trait HierarchicalLevel: TableLevel {
    type NextLevel: TableLevel;
}

pub trait AnyLevel {
    fn entries(&self) -> [Entry; ENTRY_COUNT as usize];
    fn next(&self, index: u64) -> Option<&dyn AnyLevel>;
}

impl_level_direct!(DirectLevel4 => DirectLevel3 => DirectLevel2 => DirectLevel1);
impl_level_recurse!(RecurseLevel4 => RecurseLevel3 => RecurseLevel2 => RecurseLevel1);
