use pager::address::{Frame, VirtAddr};

use super::table::{
    HierarchicalLevel, NextTableAddress, RecurseLevel1, Table, TableLevel, TableLevel4,
};
use super::{ActivePageTable, Page};
use crate::memory::paging::EntryFlags;
use crate::memory::FrameAllocator;

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

    /// # Safety
    ///
    /// The caller must ensure that the provided frame is valid and does not causes any side
    /// effects
    pub unsafe fn map<P4>(&mut self, frame: Frame, active_table: &mut ActivePageTable<P4>) -> VirtAddr
    where
        P4: HierarchicalLevel + TableLevel4,
        P4::Marker: NextTableAddress,
        P4::NextLevel: HierarchicalLevel,
        <<P4 as HierarchicalLevel>::NextLevel as TableLevel>::Marker: NextTableAddress,
        <<P4 as HierarchicalLevel>::NextLevel as HierarchicalLevel>::NextLevel: HierarchicalLevel,
        <<<P4 as HierarchicalLevel>::NextLevel as HierarchicalLevel>::NextLevel as TableLevel>::Marker:
            NextTableAddress
    {
        assert!(
            active_table.translate_page(self.page).is_none(),
            "temporary page is already mapped"
        );
        assert!(!self.mapped);

        unsafe { active_table.map_to(self.page, frame, EntryFlags::WRITABLE, &mut self.allocator) };

        self.mapped = true;
        return self.page.start_address();
    }

    /// Unmap the page from the active_table
    ///
    /// # Panics
    /// if the page is not mapped this will panic
    pub fn unmap<P4>(&mut self, active_table: &mut ActivePageTable<P4>)
    where
        P4: HierarchicalLevel + TableLevel4,
        P4::Marker: NextTableAddress,
        P4::NextLevel: HierarchicalLevel,
        <<P4 as HierarchicalLevel>::NextLevel as TableLevel>::Marker: NextTableAddress,
        <<P4 as HierarchicalLevel>::NextLevel as HierarchicalLevel>::NextLevel: HierarchicalLevel,
        <<<P4 as HierarchicalLevel>::NextLevel as HierarchicalLevel>::NextLevel as TableLevel>::Marker:
            NextTableAddress
    {
        assert!(self.mapped);
        // SAFETY: We know that the frame is valid because [`self.mapped`]
        unsafe { active_table.unmap_addr(self.page) };
        self.mapped = false;
    }

    /// # Safety
    ///
    /// The caller must ensure that the provided frame is valid and does not causes any side
    /// effects
    pub unsafe fn map_table_frame<P4>(
        &mut self,
        frame: Frame,
        active_table: &mut ActivePageTable<P4>,
    ) -> &mut Table<RecurseLevel1>
    where
       P4: HierarchicalLevel + TableLevel4,
       P4::Marker: NextTableAddress,
       P4::NextLevel: HierarchicalLevel,
       <<P4 as HierarchicalLevel>::NextLevel as TableLevel>::Marker: NextTableAddress,
       <<P4 as HierarchicalLevel>::NextLevel as HierarchicalLevel>::NextLevel: HierarchicalLevel,
       <<<P4 as HierarchicalLevel>::NextLevel as HierarchicalLevel>::NextLevel as TableLevel>::Marker:
           NextTableAddress
    {
        unsafe {
            &mut *(self
                .map(frame, active_table)
                .as_mut_ptr::<Table<RecurseLevel1>>())
        }
    }
}

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

impl FrameAllocator for TinyAllocator {
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
