use bootbridge::{MemoryMap, MemoryType};

use crate::memory::{FrameAllocator, PAGE_SIZE};

pub struct LinearAllocator {
    orginal_start: usize,
    current: usize,
    size: usize,
}

impl LinearAllocator {
    pub unsafe fn new(memory_map: &MemoryMap<'static>) -> Self {
        let mut entry = memory_map
            .entries()
            .filter(|e| {
                matches!(
                    e.ty,
                    MemoryType::CONVENTIONAL | MemoryType::BOOT_SERVICES_CODE
                )
            })
            .filter_map(|e| e.phys_align(PAGE_SIZE))
            .next()
            .expect("Failed to find free memory areas for the linear allocator");
        // Rust dosn't like null memory addresses
        entry.phys_start += PAGE_SIZE;
        Self {
            orginal_start: entry.phys_start as usize,
            current: entry.phys_start as usize,
            size: (entry.page_count * PAGE_SIZE) as usize,
        }
    }

    pub unsafe fn new_custom(start: usize, size: usize) -> Self {
        Self {
            orginal_start: start,
            current: start,
            size,
        }
    }

    pub fn original_start(&self) -> usize {
        self.orginal_start
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// You must be sure that all the allocation are no longer use
    pub unsafe fn reset(&mut self) {
        self.current = self.orginal_start;
    }
}

impl FrameAllocator for LinearAllocator {
    fn allocate_frame(&mut self) -> Option<crate::memory::Frame> {
        if self.current >= self.current + self.size {
            return None;
        }
        let addr = self.current;
        self.current += PAGE_SIZE as usize;
        return Some(crate::memory::Frame::containing_address(addr as u64));
    }
    fn deallocate_frame(&mut self, _frame: crate::memory::Frame) {}
}
