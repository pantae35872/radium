use super::{
    paging::{table::RecurseLevel4, ActivePageTable, EntryFlags, Page, PageIter},
    FrameAllocator, PAGE_SIZE,
};

pub struct StackAllocator {
    range: PageIter,
}

impl StackAllocator {
    pub fn new(page_range: PageIter) -> StackAllocator {
        StackAllocator { range: page_range }
    }
}

impl StackAllocator {
    pub fn alloc_stack<FA: FrameAllocator>(
        &mut self,
        active_table: &mut ActivePageTable<RecurseLevel4>,
        frame_allocator: &mut FA,
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
            (Some(_), Some(start), Some(end)) => {
                self.range = range;

                for page in Page::range_inclusive(start, end) {
                    active_table.map(page, EntryFlags::WRITABLE, frame_allocator);
                }

                let top_of_stack = end.start_address() + PAGE_SIZE;
                Some(Stack::new(top_of_stack, start.start_address()))
            }
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct Stack {
    top: u64,
    bottom: u64,
}

impl Stack {
    fn new(top: u64, bottom: u64) -> Stack {
        assert!(top > bottom);
        Stack { top, bottom }
    }

    pub fn top(&self) -> u64 {
        self.top
    }

    pub fn bottom(&self) -> u64 {
        self.bottom
    }
}
