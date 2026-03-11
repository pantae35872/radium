use bootbridge::{MemoryDescriptor, MemoryMap, MemoryMapIterOwned, MemoryType};
use pager::{
    PAGE_SIZE,
    address::{Frame, PageSize, PhysAddr},
    allocator::{FrameAllocator, linear_allocator::LinearAllocator},
};

use sentinel::log;

pub struct AreaAllocator<'a> {
    current_area: Option<LinearAllocator>,
    areas: MemoryMapIterOwned<'a>,
}

impl<'a> AreaAllocator<'a> {
    /// Create a new area allocator
    ///
    /// # Safety
    ///
    /// This is unsafe because we can't gurentee that the memory mapped has already been allocated
    /// to another allocator or not.
    ///
    /// and the [`FrameAllocator`] require that the allocated address is valid, and is the only
    /// owner ship of the frame
    pub unsafe fn new(memory_map: &MemoryMap<'a>) -> AreaAllocator<'a> {
        AreaAllocator { areas: memory_map.entries_owned(), current_area: None }
    }

    pub fn replace_memory_map(&mut self, memory_map: MemoryMap<'a>) {
        self.areas.replace_map(memory_map);
    }
}

impl<'a> AreaAllocator<'a> {
    fn next_filter(&mut self) -> Option<&'a MemoryDescriptor> {
        while let Some(descriptor) = self.areas.next() {
            if descriptor.ty != MemoryType::CONVENTIONAL {
                continue;
            }

            return Some(descriptor);
        }
        None
    }

    fn next_area(&mut self) {
        let Some(mut area) = self.next_filter() else {
            return;
        };
        // Reserved the first entry if null
        if area.phys_start.is_null() {
            area = match self.next_filter() {
                Some(area) => area,
                None => return,
            }
        }
        // SAFETY: This is safe because the memory map is valid, and is gurenntee by uefi and the bootloader
        self.current_area =
            Some(unsafe { LinearAllocator::new(area.phys_start, (area.page_count * PAGE_SIZE) as usize) });
    }

    pub fn allocate_entire_buffer(&mut self) -> Option<(PhysAddr, usize)> {
        if self.current_area.is_none() {
            self.next_area();
        }

        let current_area = self.current_area.as_mut()?;

        let result = (
            current_area.current(),
            current_area.size() - (current_area.current().as_u64() - current_area.original_start().as_u64()) as usize,
        );
        self.current_area = None;
        Some(result)
    }
}

unsafe impl<'a> FrameAllocator for AreaAllocator<'a> {
    fn allocate_frame<S: PageSize>(&mut self) -> Option<Frame<S>> {
        if self.current_area.is_none() {
            self.next_area();
        }

        let current_area = self.current_area.as_mut()?;

        current_area.allocate_frame::<S>().or_else(|| {
            self.current_area = None;
            self.allocate_frame()
        })
    }

    fn deallocate_frame<S: PageSize>(&mut self, frame: Frame<S>) {
        log!(Warning, "deallocate called on area allocator with frame: {:#x}", frame.start_address());
    }
}
