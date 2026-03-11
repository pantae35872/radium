use crate::address::{Frame, FrameIter, Page, PageIter, PageSize, PhysAddr, VirtAddr};

use super::FrameAllocator;

#[derive(Debug, Clone)]
pub struct LinearAllocator {
    orginal_start: PhysAddr,
    current: PhysAddr,
    size: usize,
}

impl LinearAllocator {
    /// Create a new linear allocator
    ///
    /// # Safety
    /// The caller must ensure that the provide start and size are valid and not overlap with other
    /// allocator or is being used
    pub unsafe fn new(start: PhysAddr, size: usize) -> Self {
        Self { orginal_start: start, current: start, size }
    }

    pub fn mappings(&self) -> LinearAllocatorMappings {
        LinearAllocatorMappings { start: self.orginal_start, size: self.size }
    }

    pub fn range_frame<S: PageSize>(&self) -> FrameIter<S> {
        Frame::range_inclusive(self.original_start().into(), self.end().into())
    }

    pub fn range_page<S: PageSize>(&self) -> PageIter<S> {
        Page::range_inclusive(
            VirtAddr::new(self.original_start().as_u64()).into(),
            VirtAddr::new(self.end().as_u64()).into(),
        )
    }

    pub fn original_start(&self) -> PhysAddr {
        self.orginal_start
    }

    pub fn end(&self) -> PhysAddr {
        self.orginal_start + self.size - 1
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn current(&self) -> PhysAddr {
        self.current
    }

    /// Reset the linear allocation to it's original start
    ///
    /// # Safety
    /// The caller must ensure that all the allocation are no longer use
    pub unsafe fn reset(&mut self) {
        self.current = self.orginal_start;
    }
}

impl LinearAllocatorMappings {
    pub fn start(&self) -> PhysAddr {
        self.start
    }

    pub fn end(&self) -> PhysAddr {
        self.start + self.size - 1
    }

    pub fn size(&self) -> usize {
        self.size
    }
}

// SAFETY: The ownership is guarantee by the caller of the [LinearAllocator::new] function
unsafe impl FrameAllocator for LinearAllocator {
    fn allocate_frame<S: PageSize>(&mut self) -> Option<Frame<S>> {
        if self.current >= self.end() {
            return None;
        }
        let addr = self.current;
        self.current += S::SIZE;

        Some(Frame::containing_address(addr))
    }
    fn deallocate_frame<S: PageSize>(&mut self, _frame: Frame<S>) {}
}

// This is to get around the borrow checker rules incase of mapping the same allocator using the
// same allocator
#[derive(Debug)]
pub struct LinearAllocatorMappings {
    start: PhysAddr,
    size: usize,
}
