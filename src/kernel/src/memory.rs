use x86_64::PhysAddr;

pub use self::area_frame_allocator::AreaFrameAllocator;
pub use self::paging::remap_the_kernel;

pub mod area_frame_allocator;
pub mod paging;
pub mod stack_allocator;

#[derive(PartialEq, PartialOrd, Clone)]
pub struct Frame {
    number: u64,
}

pub const PAGE_SIZE: u64 = 4096;

impl Frame {
    pub fn containing_address(address: u64) -> Frame {
        Frame {
            number: address / PAGE_SIZE,
        }
    }
    pub fn start_address(&self) -> PhysAddr {
        PhysAddr::new(self.number * PAGE_SIZE)
    }
    pub fn range_inclusive(start: Frame, end: Frame) -> FrameIter {
        FrameIter { start, end }
    }
}

pub trait FrameAllocator {
    fn allocate_frame(&mut self) -> Option<Frame>;
    fn deallocate_frame(&mut self, frame: Frame);
}

pub struct FrameIter {
    start: Frame,
    end: Frame,
}

impl Iterator for FrameIter {
    type Item = Frame;

    fn next(&mut self) -> Option<Frame> {
        if self.start <= self.end {
            let frame = self.start.clone();
            self.start.number += 1;
            Some(frame)
        } else {
            None
        }
    }
}
