use pager::{
    address::{Frame, Page, PageIter, PhysAddr, VirtAddr},
    allocator::FrameAllocator,
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
            (Some(guard), Some(start), Some(end)) => {
                self.range = range;

                for page in Page::range_inclusive(start, end) {
                    active_table.map(page, EntryFlags::WRITABLE, frame_allocator);
                }

                let top_of_stack = end.start_address().as_u64() + PAGE_SIZE;
                Some(Stack::new(
                    VirtAddr::new(top_of_stack),
                    start.start_address(),
                ))
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
