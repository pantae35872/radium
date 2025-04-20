use x86_64::VirtAddr;

use crate::{log, logger::LOGGER, serial_print, serial_println};

use super::{
    allocator::buddy_allocator::BuddyAllocator,
    paging::{table::RecurseLevel4, ActivePageTable, EntryFlags, Page, PageIter},
    Frame, FrameAllocator, PAGE_SIZE,
};

pub struct StackAllocator {
    range: PageIter,
    original_range: PageIter,
}

impl StackAllocator {
    pub fn new(page_range: PageIter) -> StackAllocator {
        StackAllocator {
            range: page_range.clone(),
            original_range: page_range,
        }
    }
}

impl StackAllocator {
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

                active_table.unmap_addr(guard);

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
    pub fn new(top: u64, bottom: u64) -> Stack {
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
