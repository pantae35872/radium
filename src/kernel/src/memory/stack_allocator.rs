use pager::{
    EntryFlags, PAGE_SIZE,
    address::{Page, PageIter, Size4K, VirtAddr},
    allocator::FrameAllocator,
    paging::{mapper::Mapper, table::RootLevel},
};
use sentinel::log;

pub struct StackAllocator {
    range: PageIter<Size4K>,
    original_range: PageIter<Size4K>,
    ua: bool,
}

impl StackAllocator {
    /// Create a new stack allocator
    pub fn new(page_range: PageIter<Size4K>, ua: bool) -> StackAllocator {
        StackAllocator { range: page_range.clone(), original_range: page_range, ua }
    }

    pub fn original_range(&self) -> PageIter<Size4K> {
        self.original_range.clone()
    }

    pub fn alloc_stack<Root: RootLevel, A: FrameAllocator>(
        &mut self,
        mapper: &mut Mapper<Root>,
        frame_allocator: &mut A,
        size_in_pages: usize,
    ) -> Option<Stack> {
        if size_in_pages == 0 {
            return None;
        }

        let mut range = self.range.clone();

        let guard_page = range.next();
        let stack_start = range.next();
        let stack_end = if size_in_pages == 1 { stack_start } else { range.nth(size_in_pages - 2) };

        match (guard_page, stack_start, stack_end) {
            (Some(_guard), Some(start), Some(end)) => {
                self.range = range;

                for page in Page::range_inclusive(start, end) {
                    mapper.map(
                        page,
                        EntryFlags::WRITABLE
                            | EntryFlags::NO_EXECUTE
                            | if self.ua { EntryFlags::USER_ACCESSIBLE } else { EntryFlags::empty() },
                        frame_allocator,
                    );
                }

                let top_of_stack = end.start_address().as_u64() + PAGE_SIZE;
                log!(
                    Trace,
                    "Allocated stack, size: {size:#x}, top: {top:#x}, bottom: {bottom:#x}",
                    bottom = start.start_address().as_u64(),
                    top = top_of_stack,
                    size = size_in_pages as u64 * PAGE_SIZE,
                );
                // SAFETY: We've already mapped the stack above as writeable and non executeable
                Some(unsafe { Stack::new(VirtAddr::new(top_of_stack), start.start_address()) })
            }
            _ => None,
        }
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
        assert!(top.as_u64().is_multiple_of(16), "stack top must be 16-byte aligned (got {:#x})", top.as_u64());
        assert!(
            bottom.as_u64().is_multiple_of(16),
            "stack bottom must be 16-byte aligned (got {:#x})",
            bottom.as_u64()
        );
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
