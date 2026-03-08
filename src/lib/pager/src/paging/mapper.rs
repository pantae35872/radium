use crate::address::{AnyFrame, AnyPage, Frame, Page, PageSize, PhysAddr, Size4K, VirtAddr};
use crate::allocator::FrameAllocator;
use crate::paging::table::entry::Entry;
use crate::paging::table::{DirectP4Create, RecurseP4Create, RootLevel, TableLevel};
use crate::registers::tlb;
use crate::{PageLevel, any_frame_select, any_page_select};

use super::EntryFlags;
use super::table::Table;
use core::ptr::NonNull;

pub struct Mapper<P4: RootLevel> {
    p4: NonNull<Table<P4>>,
}

pub struct MapperWithAllocator<'a, P4: RootLevel, A: FrameAllocator> {
    pub mapper: &'a mut Mapper<P4>,
    pub allocator: &'a mut A,
}

impl<P4> Mapper<P4>
where
    P4: RootLevel<CreateMarker = RecurseP4Create>,
{
    /// Create a mapper from the currently active recursive mapped page table
    ///
    /// # Safety
    ///
    /// The caller must ensure that the current active page table is recursive mapped
    pub unsafe fn new() -> Mapper<P4> {
        Mapper {
            // SAFETY: Whenver the current mappins is recursive or not is gurentee by the user
            p4: unsafe { P4::CreateMarker::create() },
        }
    }
}

impl<P4> Mapper<P4>
where
    P4: RootLevel<CreateMarker = DirectP4Create>,
{
    /// Create a mapper from the provided page table address
    ///
    /// # Safety
    ///
    /// The caller must ensure that the provided p4 is valid,
    /// and is the only mutable reference to the page table
    pub unsafe fn new_custom(p4: *mut Table<P4>) -> Mapper<P4> {
        Mapper {
            // SAFETY: The validity of the page table is gurentee by the caller
            p4: unsafe { P4::CreateMarker::create(p4) },
        }
    }
}

impl<P4> Mapper<P4>
where
    P4: RootLevel,
{
    pub fn p4(&self) -> &Table<P4> {
        // SAFETY: We know this is safe because we are the only one who own the active page table
        // or the actively mapping inactive page tables
        unsafe { self.p4.as_ref() }
    }

    pub fn populate_p4_lower_half(&mut self, allocator: &mut impl FrameAllocator) {
        let p4 = self.p4_mut();
        for i in 0..256 {
            p4.next_table_create(i, allocator).expect("P4 Huge page is not supported");
        }
    }

    pub fn populate_p4_upper_half(&mut self, allocator: &mut impl FrameAllocator) {
        let p4 = self.p4_mut();
        for i in 256..512 {
            p4.next_table_create(i, allocator).expect("P4 Huge page is not supported");
        }
    }

    pub fn p4_mut(&mut self) -> &mut Table<P4> {
        // SAFETY: We know this is safe because we are the only one who own the active page table
        // or the actively mapping inactive page tables
        unsafe { self.p4.as_mut() }
    }

    /// Translate the provided virtual address into the mapped physical address
    ///
    /// If the virtual address is not mapped, will return none
    pub fn translate(&self, virtual_address: VirtAddr) -> Option<PhysAddr> {
        self.translate_page(Page::<Size4K>::containing_address(virtual_address)).map(|frame| {
            PhysAddr::new(frame.start_address().as_u64() + (virtual_address.as_u64() & (frame.size() - 1)))
        })
    }

    /// Translate the provided page into the mapped frame
    ///
    /// If the page is not mapped, will return none
    pub fn translate_page<S: PageSize>(&self, page: Page<S>) -> Option<AnyFrame> {
        let p3 = self.p4().next_table(page.p4_index())?;

        fn get<L: TableLevel>(entry: &Entry<L>) -> Option<AnyFrame>
        where
            AnyFrame: From<Frame<L::PageSize>>,
        {
            if !entry.flags().contains(EntryFlags::PRESENT) {
                return None;
            }

            Some(entry.pointed_frame().expect("Invalid Entry!").erase())
        }

        let Some(p2) = p3.next_table(page.p3_index()) else {
            return get(&p3[page.p3_index() as usize]);
        };

        let Some(p1) = p2.next_table(page.p2_index()) else {
            return get(&p2[page.p2_index() as usize]);
        };

        get(&p1[page.p1_index() as usize])
    }

    /// Change the flags of the frame
    ///
    /// # Safety
    /// The caller must ensure that changing the entry flags doesn't cause any unsafe side effects
    ///
    /// # Panics
    /// Panics if the page isn't mapped,
    pub unsafe fn change_flags<S: PageSize>(&mut self, page: Page<S>, map: impl FnOnce(EntryFlags) -> EntryFlags) {
        assert!(self.translate_page(page).is_some(), "trying to change the flags of an unmapped page");

        let p3 = self.p4_mut().next_table_mut(page.p4_index()).expect("P4 can't be huge page");

        fn change<L: TableLevel, S: PageSize>(
            entry: &mut Entry<L>,
            page: Page<S>,
            map: impl FnOnce(EntryFlags) -> EntryFlags,
        ) {
            let frame = entry.pointed_frame().unwrap();
            entry.set(frame, map(entry.flags()) | EntryFlags::PRESENT);
            tlb::flush(page.start_address());
        }

        let Some(p2) = p3.next_table_mut(page.p3_index()) else {
            assert_eq!(S::LEVEL, PageLevel::Page1G, "trying to change flags of 1GiB page with {:?} page", S::LEVEL);

            change(&mut p3[page.p3_index() as usize], page, map);

            return;
        };
        let Some(p1) = p2.next_table_mut(page.p2_index()) else {
            assert_eq!(S::LEVEL, PageLevel::Page2M, "trying to change flags of 2MiB page with {:?} page", S::LEVEL);

            change(&mut p2[page.p2_index() as usize], page, map);

            return;
        };

        assert_eq!(S::LEVEL, PageLevel::Page4K, "trying to change flags of 4KiB page with {:?} page", S::LEVEL);

        change(&mut p1[page.p1_index() as usize], page, map);
    }

    /// Just a range helper See [Self::change_flags] for more info
    ///
    /// # Safety
    /// See [Self::change_flags]
    pub unsafe fn change_flags_ranges<S: PageSize>(
        &mut self,
        start_page: Page<S>,
        end_page: Page<S>,
        map: impl Fn(EntryFlags) -> EntryFlags,
    ) {
        Page::range_inclusive(start_page, end_page).for_each(|page| unsafe { self.change_flags(page, &map) });
    }

    /// Allocate a frame and map the page to the allocated frame
    ///
    /// # Panics
    /// panics if the page is already mapped
    pub fn map<A: FrameAllocator, S: PageSize>(&mut self, page: Page<S>, flags: EntryFlags, allocator: &mut A) {
        let frame = allocator.allocate_frame().expect("out of memory");
        // SAFETY: This is safe because we know that the frame is valid from the allocator
        unsafe { self.map_to(page, frame, flags, allocator) }
    }

    /// Just a range helper, See [`Self::map`] for more info
    ///
    /// # Note
    /// The range is inclusive
    pub fn map_range<A: FrameAllocator, S: PageSize>(
        &mut self,
        start_page: Page<S>,
        end_page: Page<S>,
        flags: EntryFlags,
        allocator: &mut A,
    ) {
        Page::range_inclusive(start_page, end_page).for_each(|page| self.map(page, flags, allocator));
    }

    /// Map the page to the frame (Virt -> Phys)
    ///
    /// # Safety
    ///
    /// The caller must ensure that the provided frame does not causes any unsafe side effects
    ///
    /// # Panics
    /// If the page is already mapped
    pub unsafe fn map_to<A, S>(&mut self, page: Page<S>, frame: Frame<S>, flags: EntryFlags, allocator: &mut A)
    where
        A: FrameAllocator,
        S: PageSize,
    {
        let p4 = self.p4_mut();
        let p3 = p4.next_table_create(page.p4_index(), allocator).expect("P4 huge page is unsupported");
        match S::LEVEL {
            PageLevel::Page1G => {
                p3[page.p3_index() as usize].set(frame, flags | EntryFlags::PRESENT | EntryFlags::HUGE_PAGE);
            }
            PageLevel::Page2M => {
                let p2 = p3.next_table_create(page.p3_index(), allocator).expect("P3 is already huge page mapped");

                p2[page.p2_index() as usize].set(frame, flags | EntryFlags::PRESENT | EntryFlags::HUGE_PAGE);
            }
            PageLevel::Page4K => {
                let p2 = p3.next_table_create(page.p3_index(), allocator).expect("P3 is already huge page mapped");
                let p1 = p2.next_table_create(page.p2_index(), allocator).expect("P2 is already huge page mapped");

                assert!(p1[page.p1_index() as usize].is_unused());
                p1[page.p1_index() as usize].set(frame, flags | EntryFlags::PRESENT);
            }
        }
    }

    /// Map the virtual address (start_page) -> virtual address (end_page)
    /// to a start physical address (start_frame) -> end physical address (end_frame)
    ///
    /// # Safety
    /// See [`Self::map_to`]
    ///
    /// # Panics
    ///
    /// panics if the range or part of the range is already mapped, or the frame and page ranges
    /// are not the same size
    pub unsafe fn map_to_range<A, S: PageSize>(
        &mut self,
        start_page: Page<S>,
        end_page: Page<S>,
        start_frame: Frame<S>,
        end_frame: Frame<S>,
        flags: EntryFlags,
        allocator: &mut A,
    ) where
        A: FrameAllocator,
    {
        // Check if the ranges have the same size
        assert_eq!(
            end_page.start_address().as_u64() - start_page.start_address().as_u64(),
            end_frame.start_address().as_u64() - start_frame.start_address().as_u64()
        );
        // SAFETY: it's on the user if the mapped range cause unsafe side effects or not
        Page::range_inclusive(start_page, end_page)
            .zip(Frame::range_inclusive(start_frame, end_frame))
            .for_each(|(page, frame)| unsafe { self.map_to(page, frame, flags, allocator) });
    }

    /// Identity map the provided frame
    ///
    /// # Safety
    /// The caller must ensure that the provided frame when map does not cause any unsafe side
    /// effects
    ///
    /// # Panics
    /// panics if the equivalent page is already mapped
    pub unsafe fn identity_map<A, S: PageSize>(&mut self, frame: Frame<S>, flags: EntryFlags, allocator: &mut A)
    where
        A: FrameAllocator,
    {
        let page = Page::containing_address(VirtAddr::new(frame.start_address().as_u64()));
        unsafe { self.map_to(page, frame, flags, allocator) }
    }

    /// Identity map an [AnyFrame], See [Self::identity_map] for more info
    ///
    /// # Safety
    /// See [Self::identity_map]
    pub unsafe fn identity_map_any<A, S: PageSize>(&mut self, frame: AnyFrame, flags: EntryFlags, allocator: &mut A)
    where
        A: FrameAllocator,
    {
        any_frame_select!(frame, (frame) => unsafe { self.identity_map(frame, flags, allocator) });
    }

    /// Identity map the inclusive ranges
    ///
    /// # Safety
    /// The caller must ensure that the provided frame when map does not cause any unsafe side
    /// effects
    ///
    /// # Panics
    /// panics if the range is already mapped
    pub unsafe fn identity_map_range<A, S: PageSize>(
        &mut self,
        start_frame: Frame<S>,
        end_frame: Frame<S>,
        flags: EntryFlags,
        allocator: &mut A,
    ) where
        A: FrameAllocator,
    {
        // SAFETY: it's is on the caller if this causes unsafe side effects
        Frame::range_inclusive(start_frame, end_frame)
            .for_each(|frame| unsafe { self.identity_map(frame, flags, allocator) });
    }

    /// Unmap address ranges from the page table
    ///
    /// # Safety
    /// See [`Self::unmap_addr`]
    pub unsafe fn unmap_addr_ranges<S: PageSize>(&mut self, start_page: Page<S>, end_page: Page<S>) {
        Page::range_inclusive(start_page, end_page).for_each(|page| unsafe {
            self.unmap_addr(page);
        })
    }

    /// Unmap the page from the page table and return the pointed frame
    ///
    /// # Safety
    /// and the caller must ensure that reference or allocation referencing this page no longer
    /// exists
    ///
    /// # Panics
    /// This panics if the page weren't map or the page size doesn't match with the mapped page
    pub unsafe fn unmap_addr<S: PageSize>(&mut self, page: Page<S>) -> AnyFrame {
        assert!(self.translate_page(page).is_some(), "Trying to unmap a page that weren't mapped");

        let p3 = self.p4_mut().next_table_mut(page.p4_index()).expect("P4 can't be huge page");

        fn unmap<L: TableLevel, S: PageSize>(entry: &mut Entry<L>, page: Page<S>) -> AnyFrame
        where
            AnyFrame: From<Frame<L::PageSize>>,
        {
            assert_eq!(
                L::PageSize::LEVEL,
                S::LEVEL,
                "trying to unmap {:?} page with {:?} page",
                L::PageSize::LEVEL,
                S::LEVEL
            );

            let frame = entry.pointed_frame().expect("Invalid entry state").erase();
            entry.set_unused();
            tlb::flush(page.start_address());
            frame
        }

        let Some(p2) = p3.next_table_mut(page.p3_index()) else {
            return unmap(&mut p3[page.p3_index() as usize], page);
        };
        let Some(p1) = p2.next_table_mut(page.p2_index()) else {
            return unmap(&mut p2[page.p2_index() as usize], page);
        };

        unmap(&mut p1[page.p1_index() as usize], page)
    }

    /// Unmap an [AnyPage], See [Self::unmap_addr] for more info
    ///
    /// # Safety
    /// See [Self::unmap_addr]
    pub unsafe fn unmap_addr_any<S: PageSize>(&mut self, page: AnyPage) -> AnyFrame {
        any_page_select!(page, (page) => unsafe { self.unmap_addr(page) })
    }

    /// Unmap the ranges from the page table
    ///
    /// # Safety
    /// See [`Self::unmap`]
    pub unsafe fn unmap_ranges<A, S: PageSize>(&mut self, start_page: Page<S>, end_page: Page<S>, allocator: &mut A)
    where
        A: FrameAllocator,
    {
        Page::range_inclusive(start_page, end_page).for_each(|page| unsafe { self.unmap(page, allocator) });
    }

    /// Unmap the page and deallocate it using the provided allocator
    ///
    /// # Safety
    ///
    /// The caller must ensure that the page provide was mapped by [`Self::map`],
    /// and unmapping it doesn't causes any unsafe side effects
    pub unsafe fn unmap<A, S: PageSize>(&mut self, page: Page<S>, allocator: &mut A)
    where
        A: FrameAllocator,
    {
        // SAFETY: Whever the frame is valid or not is handled by the user of this function
        allocator.deallocate_frame_any(unsafe { self.unmap_addr(page) });
    }
}
