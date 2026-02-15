use crate::address::{Frame, Page, Size4K, VirtAddr};
use crate::allocator::FrameAllocator;
use crate::paging::mapper::TopLevelP4;
use crate::{EntryFlags, PAGE_SIZE, virt_addr_alloc};

use super::ActivePageTable;
use super::table::{RecurseLevel1, Table};

/// A page that can be map and unmap, once at a time
pub struct TemporaryPage {
    mapped: bool,
    page: Page,
}

impl Default for TemporaryPage {
    fn default() -> Self {
        Self::new()
    }
}

impl TemporaryPage {
    pub fn new() -> Self {
        TemporaryPage { mapped: false, page: virt_addr_alloc(1) }
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
        allocator: &mut impl FrameAllocator,
    ) -> VirtAddr
    where
        P4: TopLevelP4,
    {
        assert!(active_table.translate_page(self.page).is_none() || self.mapped, "temporary page is already mapped");

        // SAFETY: The frame contact is uphold by the caller
        unsafe { active_table.map_to(self.page, frame, EntryFlags::WRITABLE, allocator) };

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
        assert!(self.mapped, "Trying to unmap a temporary page that is not map");
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
    pub unsafe fn map_table_frame<'a, 'b, P4, P4Access>(
        &'a mut self,
        frame: Frame<Size4K>,
        active_table: &'b mut ActivePageTable<P4>,
        allocator: &'b mut impl FrameAllocator,
    ) -> &'a mut Table<P4Access>
    where
        P4: TopLevelP4,
        P4Access: TopLevelP4,
    {
        const _: () = assert!(size_of::<Table<RecurseLevel1>>() == PAGE_SIZE as usize);
        // SAFETY: The contact is uphold by the caller, and taking a reference of a frame as a
        // table is safe because the PAGE_SIZE (which is a size of a frame) is equal to size of
        // Table<RecurseLevel1>, gurentee by const assert above
        unsafe { &mut *(self.map(frame, active_table, allocator).as_mut_ptr::<Table<P4Access>>()) }
    }
}
