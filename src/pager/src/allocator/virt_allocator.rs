use core::sync::atomic::{AtomicU64, Ordering};

use crate::{
    PAGE_SIZE,
    address::{Frame, FrameIter, Page, PageIter, PhysAddr, VirtAddr},
};

/// Same as linear allocator but allocate virtual address instead
/// and atomic too
#[derive(Debug)]
pub struct VirtualAllocator {
    orginal_start: VirtAddr,
    current: AtomicU64,
    size: usize,
}

impl VirtualAllocator {
    /// Create a new virtual allocator
    pub const fn new(start: VirtAddr, size: usize) -> Self {
        Self {
            orginal_start: start,
            current: AtomicU64::new(start.as_u64()),
            size,
        }
    }

    pub fn range_frame(&self) -> FrameIter {
        Frame::range_inclusive(
            PhysAddr::new(self.original_start().as_u64()).into(),
            PhysAddr::new(self.end().as_u64()).into(),
        )
    }

    pub fn range(&self) -> PageIter {
        Page::range_inclusive(self.original_start().into(), self.end().into())
    }

    pub fn original_start(&self) -> VirtAddr {
        self.orginal_start
    }

    pub fn end(&self) -> VirtAddr {
        self.orginal_start + self.size - 1
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn current(&self) -> VirtAddr {
        VirtAddr::new(self.current.load(Ordering::Relaxed))
    }

    pub fn allocate(&self, size_in_pages: usize) -> Option<Page> {
        assert_ne!(size_in_pages, 0);
        let current = self
            .current
            .fetch_add(PAGE_SIZE * size_in_pages as u64, Ordering::SeqCst);
        if current >= self.end().as_u64() {
            return None;
        }

        Some(Page::containing_address(VirtAddr::new(current)))
    }
}
