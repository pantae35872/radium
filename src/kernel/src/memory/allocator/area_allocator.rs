use core::marker::PhantomData;

use bootbridge::{MemoryDescriptor, MemoryMap, MemoryType};
use pager::{
    PAGE_SIZE,
    address::{Frame, PhysAddr},
    allocator::{FrameAllocator, linear_allocator::LinearAllocator},
};

use sentinel::log;

pub struct AreaAllocator<'a, I> {
    areas: I,
    current_area: Option<LinearAllocator>,
    _phantom: PhantomData<&'a I>,
}

impl<'a> AreaAllocator<'a, ()> {
    /// Create a new area allocator
    ///
    /// # Safety
    ///
    /// This is unsafe because we can't gurentee that the memory mapped has already been allocated
    /// to another allocator or not.
    ///
    /// and the [`FrameAllocator`] require that the allocated address is valid, and is the only
    /// owner ship of the frame
    pub unsafe fn new(
        areas: &'a MemoryMap,
    ) -> AreaAllocator<'a, impl Iterator<Item = &'a MemoryDescriptor>> {
        let areas = areas.entries().filter(|e| e.ty == MemoryType::CONVENTIONAL);
        AreaAllocator {
            areas,
            current_area: None,
            _phantom: PhantomData,
        }
    }
}

impl<'a, I: Iterator<Item = &'a MemoryDescriptor>> AreaAllocator<'a, I> {
    fn next_area(&mut self) {
        let mut area = match self.areas.next() {
            Some(area) => area,
            None => return,
        };
        // Reserved the first entry if null
        if area.phys_start.is_null() {
            area = match self.areas.next() {
                Some(area) => area,
                None => return,
            }
        }
        // SAFETY: This is safe because the memory map is valid, and is gurenntee by uefi and the bootloader
        self.current_area = Some(unsafe {
            LinearAllocator::new(area.phys_start, (area.page_count * PAGE_SIZE) as usize)
        });
    }

    pub fn allocate_entire_buffer(&mut self) -> Option<(PhysAddr, usize)> {
        if self.current_area.is_none() {
            self.next_area();
        }

        let current_area = self.current_area.as_mut()?;

        let result = (
            current_area.current(),
            current_area.size()
                - (current_area.current().as_u64() - current_area.original_start().as_u64())
                    as usize,
        );
        self.current_area = None;
        Some(result)
    }
}

unsafe impl<'a, I: Iterator<Item = &'a MemoryDescriptor>> FrameAllocator for AreaAllocator<'a, I> {
    fn allocate_frame(&mut self) -> Option<Frame> {
        if self.current_area.is_none() {
            self.next_area();
        }

        let current_area = self.current_area.as_mut()?;

        current_area.allocate_frame().or_else(|| {
            self.current_area = None;
            self.allocate_frame()
        })
    }

    fn deallocate_frame(&mut self, frame: Frame) {
        log!(
            Warning,
            "deallocate called on area allocator with frame: {:#x}",
            frame.start_address()
        );
    }
}
