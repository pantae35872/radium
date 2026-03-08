use crate::address::{AnyFrame, Frame, PageSize};

pub mod linear_allocator;
pub mod virt_allocator;

/// Trait respresenting a *physical* frame allocator, a frame is a page size (4KiB).
///
/// # Safety
/// The implementation of this trait must gurentee that the return allocated frame is valid and is
/// the only ownership of that physical frame
///
/// # Note
/// Deallocation are optional to implement, this trait doesn't gurrentee anything about double frees,
/// but the allocation mustn't overlap
pub unsafe trait FrameAllocator {
    /// Allocate a [`Frame`],
    /// return none if allocation are not possible eg. out of memory
    fn allocate_frame<S: PageSize>(&mut self) -> Option<Frame<S>>;

    /// Deallocate a [`Frame`]
    ///
    /// # Note
    /// the implemenation may not implement this function.
    /// and double free behaviour, freeing a frame that doesn't got allocate by this allocator
    /// is implementation defined
    fn deallocate_frame<S: PageSize>(&mut self, frame: Frame<S>);

    /// Deallocate an [AnyFrame]
    ///
    /// See [Self::deallocate_frame]
    fn deallocate_frame_any(&mut self, frame: AnyFrame) {
        match frame {
            AnyFrame::Frame4K(frame) => self.deallocate_frame(frame),
            AnyFrame::Frame2M(frame) => self.deallocate_frame(frame),
            AnyFrame::Frame1G(frame) => self.deallocate_frame(frame),
        }
    }
}
