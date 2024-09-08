use crate::memory::{Frame, FrameAllocator};
use uefi::table::boot::{MemoryDescriptor, MemoryMap, MemoryType};

pub struct AreaFrameAllocator<'a> {
    next_free_frame: Frame,
    current_area: Option<&'a MemoryDescriptor>,
    areas: &'a MemoryMap<'a>,
}

impl<'a> AreaFrameAllocator<'a> {
    pub fn new(memory_areas: &'a MemoryMap<'a>) -> AreaFrameAllocator<'a> {
        let mut allocator = AreaFrameAllocator {
            next_free_frame: Frame::containing_address(0),
            current_area: None,
            areas: memory_areas,
        };

        allocator.choose_next_area();
        allocator
    }

    fn choose_next_area(&mut self) {
        self.current_area = self
            .areas
            .entries()
            .filter(|area| {
                let address = area.phys_start + (area.page_count * 4096) - 1;
                Frame::containing_address(address) >= self.next_free_frame
                    && area.ty == MemoryType::CONVENTIONAL
            })
            .min_by_key(|area| area.phys_start);

        if let Some(area) = self.current_area {
            let start_frame = Frame::containing_address(area.phys_start);
            if self.next_free_frame < start_frame {
                self.next_free_frame = start_frame;
            }
        }
    }
}

impl<'a> FrameAllocator for AreaFrameAllocator<'a> {
    fn allocate_frame(&mut self) -> Option<Frame> {
        if let Some(area) = self.current_area {
            let frame = Frame {
                number: self.next_free_frame.number,
            };

            let current_area_last_frame = {
                let address = area.phys_start + area.page_count * 4096 - 1;
                Frame::containing_address(address)
            };

            if frame > current_area_last_frame {
                self.choose_next_area();
            } else {
                self.next_free_frame.number += 1;
                return Some(frame);
            }
            self.allocate_frame()
        } else {
            None
        }
    }

    fn deallocate_frame(&mut self, _frame: Frame) {}
}
