use pager::{
    address::{Frame, PageIter, PhysAddr, VirtAddr},
    paging::{table::RecurseLevel4, ActivePageTable},
    EntryFlags, IdentityMappable, PAGE_SIZE,
};

use super::WithTable;

pub struct StackAllocator {
    range: PageIter,
    original_range: PageIter,
}

impl StackAllocator {
    /// Create a new stack allocator
    ///
    /// # Safety
    ///
    /// the caller must ensure that the provided page ranges is valid
    ///
    /// The caller must ensure that the provided page was mapped by [`crate::memory::paging::Mapper<T>::map_to`] or [`crate::memory::paging::ActivePageTable<T>::identity_map`]
    pub unsafe fn new(page_range: PageIter) -> StackAllocator {
        StackAllocator {
            range: page_range.clone(),
            original_range: page_range,
        }
    }

    pub fn original_range(&self) -> PageIter {
        self.original_range.clone()
    }

    pub fn alloc_stack(
        &mut self,
        active_table: &mut ActivePageTable<RecurseLevel4>,
        size_in_pages: usize,
    ) -> Option<Stack> {
        if size_in_pages == 0 {
            return None;
        }

        let mut range = self.range.clone();

        let guard_page = range.next();
        let stack_start = range.next();
        let stack_end = if size_in_pages == 1 {
            stack_start
        } else {
            range.nth(size_in_pages - 2)
        };

        match (guard_page, stack_start, stack_end) {
            (Some(guard), Some(start), Some(end)) => {
                self.range = range;

                // SAFETY: We're gurentee
                unsafe { active_table.unmap_addr(guard) };

                let top_of_stack = end.start_address().as_u64() + PAGE_SIZE;
                Some(Stack::new(
                    VirtAddr::new(top_of_stack),
                    start.start_address(),
                ))
            }
            _ => None,
        }
    }

    pub fn with_table<'a>(
        &'a mut self,
        active_table: &'a mut ActivePageTable<RecurseLevel4>,
    ) -> WithTable<'a, Self> {
        WithTable {
            table: active_table,
            with_table: self,
        }
    }
}

impl WithTable<'_, StackAllocator> {
    pub fn alloc_stack(&mut self, size_in_pages: usize) -> Option<Stack> {
        self.with_table.alloc_stack(self.table, size_in_pages)
    }
}

impl IdentityMappable for StackAllocator {
    fn map(&self, mapper: &mut impl pager::Mapper) {
        self.original_range().for_each(|e| unsafe {
            mapper.identity_map(
                Frame::containing_address(PhysAddr::new(e.start_address().as_u64())),
                EntryFlags::WRITABLE,
            );
        });
    }
}

#[derive(Debug)]
pub struct Stack {
    top: VirtAddr,
    bottom: VirtAddr,
}

impl Stack {
    pub fn new(top: VirtAddr, bottom: VirtAddr) -> Stack {
        assert!(top > bottom);
        Stack { top, bottom }
    }

    pub fn top(&self) -> VirtAddr {
        self.top
    }

    pub fn bottom(&self) -> VirtAddr {
        self.bottom
    }
}
