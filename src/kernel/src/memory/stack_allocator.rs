use pager::{
    address::{Page, PageIter, VirtAddr},
    allocator::FrameAllocator,
    paging::{table::RecurseLevel4, ActivePageTable},
    EntryFlags, PAGE_SIZE,
};

use super::WithTable;

pub struct StackAllocator {
    range: PageIter,
    original_range: PageIter,
}

impl StackAllocator {
    /// Create a new stack allocator
    pub fn new(page_range: PageIter) -> StackAllocator {
        StackAllocator {
            range: page_range.clone(),
            original_range: page_range,
        }
    }

    pub fn original_range(&self) -> PageIter {
        self.original_range.clone()
    }

    pub fn alloc_stack<A: FrameAllocator>(
        &mut self,
        active_table: &mut ActivePageTable<RecurseLevel4>,
        frame_allocator: &mut A,
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
            (Some(_guard), Some(start), Some(end)) => {
                self.range = range;

                for page in Page::range_inclusive(start, end) {
                    active_table.map(
                        page,
                        EntryFlags::WRITABLE | EntryFlags::NO_EXECUTE,
                        frame_allocator,
                    );
                }

                let top_of_stack = end.start_address().as_u64() + PAGE_SIZE;
                // SAFETY: We've already mapped the stack above as writeable and non executeable
                Some(unsafe { Stack::new(VirtAddr::new(top_of_stack), start.start_address()) })
            }
            _ => None,
        }
    }

    pub fn with_table<'a, A: FrameAllocator>(
        &'a mut self,
        active_table: &'a mut ActivePageTable<RecurseLevel4>,
        allocator: &'a mut A,
    ) -> WithTable<'a, Self, A> {
        WithTable {
            table: active_table,
            with_table: self,
            allocator,
        }
    }
}

impl<A: FrameAllocator> WithTable<'_, StackAllocator, A> {
    pub fn alloc_stack(&mut self, size_in_pages: usize) -> Option<Stack> {
        self.with_table
            .alloc_stack(self.table, self.allocator, size_in_pages)
    }
}

/// A data structure that contains the stack value such as top and bottom
/// # Note
/// The stack guard page is optional, but the stack must be **writeable and non executeable**
#[derive(Debug)]
pub struct Stack {
    top: VirtAddr,
    bottom: VirtAddr,
}

impl Stack {
    /// Create a new stack with the provided top and bottom
    /// # Safety
    /// The caller must ensure that the provided top and bottom is **correctly allocate**,
    /// and marked as **writeable and non executeable**
    pub unsafe fn new(top: VirtAddr, bottom: VirtAddr) -> Stack {
        assert!(top > bottom);
        Stack { top, bottom }
    }

    /// Get the stack top
    /// The return [`VirtAddr`] is gurrentee to be valid and **writeable and non executeable**
    pub fn top(&self) -> VirtAddr {
        self.top
    }

    /// Get the stack bottom
    /// The return [`VirtAddr`] is gurrentee to be valid and **writeable and non executeable**
    pub fn bottom(&self) -> VirtAddr {
        self.bottom
    }
}
