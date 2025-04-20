use core::marker::PhantomData;

use bootbridge::{MemoryDescriptor, MemoryMap, MemoryMapIter, MemoryType};

use crate::{
    log,
    memory::{FrameAllocator, PAGE_SIZE},
};

use super::linear_allocator::LinearAllocator;

pub struct AreaAllocator<'a, I> {
    areas: I,
    current_area: Option<LinearAllocator>,
    _phantom: PhantomData<&'a I>,
}

impl<'a> AreaAllocator<'a, ()> {
    pub fn new(
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
    pub fn next_area(&mut self) {
        let mut area = match self.areas.next() {
            Some(area) => area,
            None => return,
        }
        .clone();
        if area.phys_start == 0 {
            area.phys_start += PAGE_SIZE;
            area.page_count -= 1;
        }
        self.current_area = Some(unsafe {
            LinearAllocator::new(
                area.phys_start as usize,
                (area.page_count * PAGE_SIZE) as usize,
            )
        });
    }

    pub fn allocate_entire_buffer(&mut self) -> Option<(usize, usize)> {
        if self.current_area.is_none() {
            self.next_area();
        }

        let current_area = match self.current_area.as_mut() {
            Some(area) => area,
            None => return None,
        };

        let result = (
            current_area.current(),
            current_area.size() - (current_area.current() - current_area.original_start()),
        );
        self.current_area = None;
        Some(result)
    }
}

impl<'a, I: Iterator<Item = &'a MemoryDescriptor>> FrameAllocator for AreaAllocator<'a, I> {
    fn allocate_frame(&mut self) -> Option<crate::memory::Frame> {
        if self.current_area.is_none() {
            self.next_area();
        }

        let current_area = match self.current_area.as_mut() {
            Some(area) => area,
            None => return None,
        };

        current_area.allocate_frame().or_else(|| {
            self.current_area = None;
            self.allocate_frame()
        })
    }

    fn deallocate_frame(&mut self, frame: crate::memory::Frame) {
        log!(
            Warning,
            "deallocate called on area allocator with frame: {:#x}",
            frame.start_address()
        );
    }
}
