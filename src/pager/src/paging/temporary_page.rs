use crate::address::{Frame, Page, VirtAddr};
use crate::allocator::FrameAllocator;
use crate::paging::mapper::TopLevelP4;
use crate::{EntryFlags, PAGE_SIZE};

use super::ActivePageTable;
use super::table::{RecurseLevel1, Table};

/// A page that can be map and unmap, once at a time
pub struct TemporaryPage {
    mapped: bool,
    page: Page,
    allocator: TinyAllocator,
}

impl TemporaryPage {
    pub fn new<A>(page: Page, allocator: &mut A) -> TemporaryPage
    where
        A: FrameAllocator,
    {
        TemporaryPage {
            mapped: false,
            page,
            allocator: TinyAllocator::new(allocator),
        }
    }

    /// Map the temporary page
    ///
    /// # Safety
    /// The caller must ensure that the provided frame is valid and does not causes any side
    /// effects
    ///
    /// # Panics
    /// Panics if the page is already mapped
    pub unsafe fn map<P4>(
        &mut self,
        frame: Frame,
        active_table: &mut ActivePageTable<P4>,
    ) -> VirtAddr
    where
        P4: TopLevelP4,
    {
        assert!(
            active_table.translate_page(self.page).is_none() || self.mapped,
            "temporary page is already mapped"
        );

        // SAFETY: The frame contact is uphold by the caller
        unsafe { active_table.map_to(self.page, frame, EntryFlags::WRITABLE, &mut self.allocator) };

        self.mapped = true;
        self.page.start_address()
    }

    /// Unmap the page from the active_table
    ///
    /// # Panics
    /// if the page is not mapped this will panic
    ///
    /// # Safety
    /// the caller must ensure that any reference to the mapped page no longer exists
    pub unsafe fn unmap<P4>(&mut self, active_table: &mut ActivePageTable<P4>)
    where
        P4: TopLevelP4,
    {
        assert!(
            self.mapped,
            "Trying to unmap a temporary page that is not map"
        );
        // SAFETY: function above use map_to and we have assertion above, so the first contact is uphold,
        // the second contact is uphold by the caller
        unsafe { active_table.unmap_addr(self.page) };
        self.mapped = false;
    }

    /// Map table and take a reference as a Table<RecurseLevel1>
    ///
    /// # Safety
    /// The caller must ensure that the provided frame is valid and does not causes any side
    /// effects
    pub unsafe fn map_table_frame<P4>(
        &mut self,
        frame: Frame,
        active_table: &mut ActivePageTable<P4>,
    ) -> &mut Table<RecurseLevel1>
    where
        P4: TopLevelP4,
    {
        const _: () = assert!(size_of::<Table<RecurseLevel1>>() == PAGE_SIZE as usize);
        // SAFETY: The contact is uphold by the caller, and taking a reference of a frame as a
        // table is safe because the PAGE_SIZE (which is a size of a frame) is equal to size of
        // Table<RecurseLevel1>, gurentee by const assert above
        unsafe {
            &mut *(self
                .map(frame, active_table)
                .as_mut_ptr::<Table<RecurseLevel1>>())
        }
    }
}

/// A simple [`FrameAllocator`] implementation that can hold 3 frame at a time
struct TinyAllocator([Option<Frame>; 3]);

impl TinyAllocator {
    fn new<A>(allocator: &mut A) -> TinyAllocator
    where
        A: FrameAllocator,
    {
        let mut f = || allocator.allocate_frame();
        let frames = [f(), f(), f()];
        TinyAllocator(frames)
    }
}

// SAFETY: since we call another frame allocator to allocate a frame, this is safe
unsafe impl FrameAllocator for TinyAllocator {
    fn allocate_frame(&mut self) -> Option<Frame> {
        for frame_option in &mut self.0 {
            if frame_option.is_some() {
                return frame_option.take();
            }
        }
        None
    }

    fn deallocate_frame(&mut self, frame: Frame) {
        for frame_option in &mut self.0 {
            if frame_option.is_none() {
                *frame_option = Some(frame);
                return;
            }
        }
        panic!("Tiny allocator can hold only 3 frames.");
    }
}
