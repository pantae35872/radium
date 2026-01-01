use linear_allocator::LinearAllocator;

use crate::{PAGE_SIZE, address::Frame};

pub mod linear_allocator;
pub mod virt_allocator;

/// Trait respresenting a frame allocator, a frame is a page size (4KiB).
///
/// # Safety
/// The implementation of this trait must gurentee that the return allocated frame is valid and is
/// the only ownership of that physical frame
///
/// # Note
/// Deallocation are optional to implement, this trait doesn't gurrentee anything about double frees,
/// but the allocation mustn't overlap
pub unsafe trait FrameAllocator {
    /// call [`FrameAllocator::allocate_frame`] until the frame allocated amount is large enough to
    /// construct a [`LinearAllocator`] with a size of `size_in_frames` out of it
    /// linear allocator from the page allocator
    ///
    /// # Note
    /// If the allocate_frame return [None] while the amount is not large enough to construct a
    /// [`LinearAllocator`], this will not call deallocate, it'll just return [None]
    fn linear_allocator(&mut self, size_in_frames: u64) -> Option<LinearAllocator> {
        let mut last_address = 0;
        let mut counter = size_in_frames;
        let mut start_frame = Frame::null();
        loop {
            let frame = self.allocate_frame()?;
            if start_frame.start_address().as_u64() == 0 {
                start_frame = frame;
            }
            // If the memory is not contiguous, reset the counter
            if last_address + PAGE_SIZE != frame.start_address().as_u64() && last_address != 0 {
                counter = size_in_frames;
                start_frame = frame;
            }
            last_address = frame.start_address().as_u64();
            counter -= 1;
            if counter == 0 {
                break;
            }
        }
        assert!(start_frame.start_address().as_u64() != 0);
        // SAFETY: We know that the frame allocator is valid
        Some(unsafe { LinearAllocator::new(start_frame.start_address(), (size_in_frames * PAGE_SIZE) as usize) })
    }

    /// Allocate a [`Frame`],
    /// return none if allocation are not possible eg. out of memory
    fn allocate_frame(&mut self) -> Option<Frame>;

    /// Deallocate a [`Frame`]
    ///
    /// # Note
    /// the implemenation may not implement this function.
    /// and the behaviour on double free, freeing a frame that doesn't allocate by this allocator
    /// is up to the implementation
    fn deallocate_frame(&mut self, frame: Frame);
}
