use bootbridge::{MemoryMap, MemoryType};

use crate::{
    log,
    memory::{FrameAllocator, PAGE_SIZE},
    smp::{TRAMPOLINE_END, TRAMPOLINE_START},
};

#[derive(Debug)]
pub struct LinearAllocator {
    orginal_start: usize,
    current: usize,
    size: usize,
}

impl LinearAllocator {
    /// Create a new linear allocator
    ///
    /// # Safety
    /// The caller must ensure that the provide start and size are valid
    pub unsafe fn new(start: usize, size: usize) -> Self {
        Self {
            orginal_start: start,
            current: start,
            size,
        }
    }

    pub fn original_start(&self) -> usize {
        self.orginal_start
    }

    pub fn end(&self) -> usize {
        self.orginal_start + self.size - 1
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn current(&self) -> usize {
        self.current
    }

    /// You must be sure that all the allocation are no longer use
    pub unsafe fn reset(&mut self) {
        self.current = self.orginal_start;
    }
}

impl FrameAllocator for LinearAllocator {
    fn allocate_frame(&mut self) -> Option<crate::memory::Frame> {
        if self.current >= self.end() {
            return None;
        }
        let addr = self.current;
        self.current += PAGE_SIZE as usize;
        // TODO: Find a better way to handle reserve areas
        if self.current >= TRAMPOLINE_START && self.current <= TRAMPOLINE_END {
            self.current = TRAMPOLINE_END.min(self.size) + PAGE_SIZE as usize;
        }
        return Some(crate::memory::Frame::containing_address(addr as u64));
    }
    fn deallocate_frame(&mut self, _frame: crate::memory::Frame) {}
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use hashbrown::HashSet;

    use super::*;

    #[test_case]
    fn allocations_are_within_bounds_and_skip_trampoline() {
        // set up a 10‐page region starting at 0
        let start = 0;
        let size = PAGE_SIZE as usize * 10;
        let mut alloc = unsafe { LinearAllocator::new(start, size) };

        // collect all allocated frame addresses
        let mut addrs = Vec::new();
        while let Some(frame) = alloc.allocate_frame() {
            let addr = frame.start_address().as_u64() as usize;
            addrs.push(addr);
        }

        // after exhausting, allocate_frame() must return None
        assert!(alloc.allocate_frame().is_none());

        // each address must:
        //  • be ≥ start
        //  • have its PAGE_SIZE block fully inside [start, start+size)
        //  • not lie in the inclusive [TRAMPOLINE_START, TRAMPOLINE_END]
        for &addr in &addrs {
            assert!(addr >= start, "addr {:x} is below start {:x}", addr, start);
            assert!(
                addr + PAGE_SIZE as usize <= start + size,
                "addr {:x} + PAGE_SIZE exceeds end {:x}",
                addr,
                start + size
            );
            assert!(
                addr < TRAMPOLINE_START || addr > TRAMPOLINE_END,
                "addr {:x} overlaps trampoline [{:x}, {:x}]",
                addr,
                TRAMPOLINE_START,
                TRAMPOLINE_END
            );
        }

        // ensure no duplicates
        let unique: HashSet<_> = addrs.iter().cloned().collect();
        assert_eq!(
            unique.len(),
            addrs.len(),
            "found {} duplicates in allocations",
            addrs.len() - unique.len()
        );
    }

    fn collect_frame_addrs(start: usize, size: usize) -> Vec<usize> {
        let mut alloc = unsafe { LinearAllocator::new(start, size) };
        let mut addrs = Vec::new();
        while let Some(frame) = alloc.allocate_frame() {
            let addr = frame.start_address().as_u64() as usize;
            addrs.push(addr);
        }
        addrs
    }

    #[test_case]
    fn allocations_are_within_bounds_skip_trampoline_and_no_overflow() {
        // set up a 10‐page region starting at 0x0
        let start = 0;
        let size = PAGE_SIZE as usize * 10;
        let mut alloc = unsafe { LinearAllocator::new(start, size) };

        // compute the absolute byte after the last valid address
        let region_end = alloc
            .original_start()
            .checked_add(alloc.size())
            .expect("overflow computing region end");

        // collect all allocated frame addresses
        let mut addrs = Vec::new();
        while let Some(frame) = alloc.allocate_frame() {
            let addr = frame.start_address().as_u64() as usize;
            addrs.push(addr);
        }

        // after exhausting, allocate_frame() must return None
        assert!(alloc.allocate_frame().is_none());

        for &addr in &addrs {
            // 1) no overflow when adding a page
            let next_page_end = addr
                .checked_add(PAGE_SIZE as usize)
                .expect("overflow when computing addr + PAGE_SIZE");

            // 2) page must fully fit within [start, start+size)
            assert!(
                next_page_end <= region_end,
                "addr {:#x} + PAGE_SIZE = {:#x} exceeds region end {:#x}",
                addr,
                next_page_end,
                region_end
            );

            // 3) skip the trampoline range entirely
            assert!(
                addr < TRAMPOLINE_START || addr > TRAMPOLINE_END,
                "addr {:#x} overlaps trampoline [{:#x}, {:#x}]",
                addr,
                TRAMPOLINE_START,
                TRAMPOLINE_END
            );
        }

        // ensure no duplicates
        let unique: HashSet<_> = addrs.iter().cloned().collect();
        assert_eq!(
            unique.len(),
            addrs.len(),
            "found {} duplicate frames",
            addrs.len() - unique.len()
        );
    }

    #[test_case]
    fn reset_restores_to_original_start() {
        let start = 0x1000;
        let size = PAGE_SIZE as usize * 3;
        let mut alloc = unsafe { LinearAllocator::new(start, size) };

        // exhaust
        while alloc.allocate_frame().is_some() {}

        // reset and verify we can allocate again at `start`
        unsafe { alloc.reset() };
        assert_eq!(alloc.current(), start);
        let first = alloc
            .allocate_frame()
            .expect("should get first frame after reset");
        let first_addr = first.start_address().as_u64() as usize;
        assert_eq!(first_addr, start);
    }
}
