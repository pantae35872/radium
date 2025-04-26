use linear_allocator::LinearAllocator;

use crate::{address::Frame, PAGE_SIZE};

pub mod linear_allocator;

pub trait FrameAllocator {
    fn linear_allocator(&mut self, size_in_frames: u64) -> Option<LinearAllocator> {
        let mut last_address = 0;
        let mut counter = size_in_frames;
        let mut start_frame = Frame::null();
        loop {
            let frame = match self.allocate_frame() {
                Some(frame) => frame,
                None => return None,
            };
            if start_frame.start_address().as_u64() == 0 {
                start_frame = frame.clone();
            }
            // If the memory is not contiguous, reset the counter
            if last_address + PAGE_SIZE != frame.start_address().as_u64() && last_address != 0 {
                counter = size_in_frames;
                start_frame = frame.clone();
            }
            last_address = frame.start_address().as_u64();
            counter -= 1;
            if counter == 0 {
                break;
            }
        }
        assert!(start_frame.start_address().as_u64() != 0);
        // We know that the frame allocator is valid
        Some(unsafe {
            LinearAllocator::new(
                start_frame.start_address(),
                (size_in_frames * PAGE_SIZE) as usize,
            )
        })
    }

    // SAFETY: The implementor of this function must gurentee that the return frame is valid and is
    // the only ownership of that physical frame
    fn allocate_frame(&mut self) -> Option<Frame>;

    fn deallocate_frame(&mut self, frame: Frame);
}
