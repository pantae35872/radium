use pager::address::{Frame, FrameIter, Page, PhysAddr, VirtAddr};
use pager::{tlb, IdentityMappable, PAGE_SIZE};

use super::table::{
    DirectP4Create, HierarchicalLevel, NextTableAddress, RecurseP4Create, Table, TableLevel,
    TableLevel4,
};
use super::{EntryFlags, ENTRY_COUNT};
use crate::log;
use crate::memory::FrameAllocator;
use core::ptr::Unique;

pub struct Mapper<P4: TableLevel4> {
    p4: Unique<Table<P4>>,
}

pub struct MapperWithAllocator<'a, P4: TableLevel4, A: FrameAllocator> {
    mapper: &'a mut Mapper<P4>,
    allocator: &'a mut A,
}

impl<P4> Mapper<P4>
where
    P4: TableLevel4,
    P4::CreateMarker: RecurseP4Create<P4>,
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
    P4: TableLevel4,
    P4::CreateMarker: DirectP4Create<P4>,
{
    /// Create a mapper from the provided page table address
    ///
    /// # Safety
    ///
    /// The caller must ensure that the provided p4 is valid,
    /// and is the only mutable reference to the page table
    pub unsafe fn new_custom(p4: *mut Table<P4>) -> Mapper<P4> {
        Mapper {
            // SAFETY: The validity of the page table is gurentee by the user
            p4: unsafe { P4::CreateMarker::create(p4) },
        }
    }
}

// Zero-cost ahhh abstraction
impl<P4> Mapper<P4>
where
    P4: HierarchicalLevel + TableLevel4,
    P4::Marker: NextTableAddress,
    P4::NextLevel: HierarchicalLevel,
    <<P4 as HierarchicalLevel>::NextLevel as TableLevel>::Marker: NextTableAddress,
    <<P4 as HierarchicalLevel>::NextLevel as HierarchicalLevel>::NextLevel: HierarchicalLevel,
    <<<P4 as HierarchicalLevel>::NextLevel as HierarchicalLevel>::NextLevel as TableLevel>::Marker:
        NextTableAddress,
{
    pub fn p4(&self) -> &Table<P4> {
        // SAFETY: We know this is safe because we are the only one who own the active page table
        // or the actively mapping inactive page tables
        unsafe { self.p4.as_ref() }
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
        let offset = virtual_address.as_u64() % PAGE_SIZE;
        return self
            .translate_page(Page::containing_address(virtual_address))
            .map(|frame| PhysAddr::new(frame.start_address().as_u64() + offset));
    }

    /// Translate the provided page into the mapped frame
    ///
    /// If the page is not mapped, will return none
    pub fn translate_page(&self, page: Page) -> Option<Frame> {
        let p3 = self.p4().next_table(page.p4_index());

        let huge_page = || {
            p3.and_then(|p3| {
                let p3_entry = &p3[page.p3_index() as usize];
                // 1GiB page?
                if let Some(start_frame) = p3_entry.pointed_frame() {
                    if p3_entry.flags().contains(EntryFlags::HUGE_PAGE) {
                        // address must be 1GiB aligned
                        assert!(start_frame.number() % (ENTRY_COUNT * ENTRY_COUNT) == 0);
                        return Some(
                            start_frame
                                .add_by_page(page.p2_index() * ENTRY_COUNT)
                                .add_by_page(page.p1_index()),
                        );
                    }
                }
                if let Some(p2) = p3.next_table(page.p3_index()) {
                    let p2_entry = &p2[page.p2_index() as usize];
                    if let Some(start_frame) = p2_entry.pointed_frame() {
                        if p2_entry.flags().contains(EntryFlags::HUGE_PAGE) {
                            assert!(start_frame.number() % ENTRY_COUNT == 0);
                            return Some(start_frame.add_by_page(page.p1_index()));
                        }
                    }
                }
                None
            })
        };

        p3.and_then(|p3| p3.next_table(page.p3_index()))
            .and_then(|p2| p2.next_table(page.p2_index()))
            .and_then(|p1| p1[page.p1_index() as usize].pointed_frame())
            .or_else(huge_page)
    }

    /// Map the page to the frame (Virt -> Phys)
    ///
    /// # Safety
    ///
    /// The caller must ensure that the provided frame does not causes any unsafe side effects
    ///
    /// # Panics
    ///
    /// The caller must ensure that the frame will not overwrite any other pages otherwise panic
    /// if the frame has been map with OVERWRITEABLE flags this will not panic
    pub unsafe fn map_to<A>(
        &mut self,
        page: Page,
        frame: Frame,
        flags: EntryFlags,
        allocator: &mut A,
    ) where
        A: FrameAllocator,
    {
        let p4 = self.p4_mut();
        let p3 = p4.next_table_create(page.p4_index(), allocator);
        let p2 = p3.next_table_create(page.p3_index(), allocator);
        let p1 = p2.next_table_create(page.p2_index(), allocator);
        if !p1[page.p1_index() as usize].is_unused() {
            log!(
                Error,
                "Trying to map to a used frame, Page {:#x}, Frame: {:#x}",
                page.start_address(),
                p1[page.p1_index() as usize]
                    .pointed_frame()
                    .unwrap()
                    .start_address()
            );
            log!(Error, "Trying to map: {:x?}", p1[page.p1_index() as usize]);
        }
        assert!(
            p1[page.p1_index() as usize].is_unused()
                || p1[page.p1_index() as usize].overwriteable()
        );
        p1[page.p1_index() as usize].set(frame, flags | EntryFlags::PRESENT);
    }

    /// Allocate a frame and map the page to the allocated frame
    ///
    /// # Panics
    /// panics if the page is already mapped and not marked OVERWRITEABLE
    pub fn map<A>(&mut self, page: Page, flags: EntryFlags, allocator: &mut A)
    where
        A: FrameAllocator,
    {
        let frame = allocator.allocate_frame().expect("out of memory");
        // SAFETY: This is safe because we know that the frame is valid from the allocator
        unsafe { self.map_to(page, frame, flags, allocator) }
    }

    /// Allocate a frames and map the ranges to the allocated frame
    ///
    /// # Note
    ///
    /// The range is inclusive
    ///
    /// # Panics
    ///
    /// panics if the range is already mapped and not marked OVERWRITEABLE
    pub fn map_range<A>(
        &mut self,
        start_page: Page,
        end_page: Page,
        flags: EntryFlags,
        allocator: &mut A,
    ) where
        A: FrameAllocator,
    {
        assert!(start_page <= end_page);
        Page::range_inclusive(start_page, end_page)
            .for_each(|page| self.map(page, flags, allocator));
    }

    /// Map the virtual address (start_page) -> virtual address (end_page)
    /// to a start physical address (start_frame) -> end physical address (end_frame)
    ///
    /// # Assertions
    /// start_page -> end_page must be contigous
    /// start_frame -> end_frame must be contigous
    /// length between (start_page -> end_page).length = (start_frame -> end_frame).length must be
    /// equal
    ///
    /// # Panics
    ///
    /// panics if the range is already mapped and not marked OVERWRITEABLE
    pub unsafe fn map_to_range<A>(
        &mut self,
        start_page: Page,
        end_page: Page,
        start_frame: Frame,
        end_frame: Frame,
        flags: EntryFlags,
        allocator: &mut A,
    ) where
        A: FrameAllocator,
    {
        assert!(start_page <= end_page);
        assert!(start_frame <= end_frame);
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

    /// Identity map the frame provided
    ///
    /// # Safety
    /// The caller must ensure that the provided frame when map does not cause any unsafe side
    /// effects
    ///
    /// # Panics
    /// panics if the page is already mapped and not marked OVERWRITEABLE
    pub unsafe fn identity_map<A>(&mut self, frame: Frame, flags: EntryFlags, allocator: &mut A)
    where
        A: FrameAllocator,
    {
        let page = Page::containing_address(VirtAddr::new(frame.start_address().as_u64()));
        unsafe { self.map_to(page, frame, flags, allocator) }
    }

    pub fn identity_map_object<O: IdentityMappable, A: FrameAllocator>(
        &mut self,
        obj: &O,
        allocator: &mut A,
    ) {
        let mut mapper = self.mapper_with_allocator(allocator);
        obj.map(&mut mapper);
    }

    pub fn mapper_with_allocator<'a, A: FrameAllocator>(
        &'a mut self,
        allocator: &'a mut A,
    ) -> MapperWithAllocator<'a, P4, A> {
        MapperWithAllocator {
            mapper: self,
            allocator,
        }
    }

    /// Identity map the inclusive ranges
    ///
    /// # Safety
    ///
    /// The caller must ensure that the provided frame when map does not cause any unsafe side
    /// effects
    ///
    /// # Panics
    ///
    /// panics if the range is already mapped and not marked OVERWRITEABLE
    pub unsafe fn identity_map_range<A>(
        &mut self,
        start_frame: Frame,
        end_frame: Frame,
        flags: EntryFlags,
        allocator: &mut A,
    ) where
        A: FrameAllocator,
    {
        // SAFETY: it's is on the caller if this causes unsafe side effects
        Frame::range_inclusive(start_frame, end_frame)
            .for_each(|frame| unsafe { self.identity_map(frame, flags, allocator) });
    }

    /// Unmap the ranges from the page table
    ///
    /// The start_page -> end_page (inclusive) must be contigous
    /// start_page != end_page
    /// end_page > start_page
    ///
    /// # Safety
    ///
    /// The caller must ensure that the provided page was mapped by [`Self::map_to_range`] or [`Self::identity_map_range`]
    pub unsafe fn unmap_addr_ranges(&mut self, start_page: Page, end_page: Page) -> FrameIter {
        assert!(start_page < end_page);
        let mut iter = Page::range_inclusive(start_page, end_page)
            .map(|page| unsafe { self.unmap_addr(page) });
        Frame::range_inclusive(iter.next().expect(""), iter.last().expect(""))
    }

    /// Unmap the ranges from the page table
    /// and deallocates from the buffer
    ///
    /// The start_page -> end_page (inclusive) must be contigous
    /// start_page != end_page
    /// end_page > start_page
    ///
    /// # Safety
    ///
    /// The caller must ensure that the provided page was mapped by [`Self::map_range`]
    pub unsafe fn unmap_ranges<A>(&mut self, start_page: Page, end_page: Page, allocator: &mut A)
    where
        A: FrameAllocator,
    {
        Page::range_inclusive(start_page, end_page)
            .for_each(|page| unsafe { self.unmap(page, allocator) });
    }

    /// Unmap the page from the page table and return the pointed frame
    ///
    /// # Safety
    ///
    /// The caller must ensure that the provided page was mapped by [`Self::map_to`] or [`Self::identity_map`]
    pub unsafe fn unmap_addr(&mut self, page: Page) -> Frame {
        assert!(self.translate(page.start_address()).is_some());

        let p1 = self
            .p4_mut()
            .next_table_mut(page.p4_index())
            .and_then(|p3| p3.next_table_mut(page.p3_index()))
            .and_then(|p2| p2.next_table_mut(page.p2_index()))
            .expect("mapping code does not support huge pages");
        let frame = p1[page.p1_index() as usize].pointed_frame().unwrap();
        p1[page.p1_index() as usize].set_unused();
        tlb::flush(page.start_address());
        frame
    }

    /// Unmap the page mapped by the map function
    ///
    /// # Safety
    ///
    /// The caller must ensure that the page provide was mapped by [`Self::map`]
    /// and must not causes any unsafe side effects
    pub unsafe fn unmap<A>(&mut self, page: Page, allocator: &mut A)
    where
        A: FrameAllocator,
    {
        // SAFETY: Whever the frame is valid or not is handled by the user of this function
        allocator.deallocate_frame(unsafe { self.unmap_addr(page) });
    }
}

impl<'a, P4, A: FrameAllocator> pager::Mapper for MapperWithAllocator<'a, P4, A>
where
    P4: HierarchicalLevel + TableLevel4,
    P4::Marker: NextTableAddress,
    P4::NextLevel: HierarchicalLevel,
    <<P4 as HierarchicalLevel>::NextLevel as TableLevel>::Marker: NextTableAddress,
    <<P4 as HierarchicalLevel>::NextLevel as HierarchicalLevel>::NextLevel: HierarchicalLevel,
    <<<P4 as HierarchicalLevel>::NextLevel as HierarchicalLevel>::NextLevel as TableLevel>::Marker:
        NextTableAddress,
{
    unsafe fn identity_map_range(
        &mut self,
        start_frame: Frame,
        end_frame: Frame,
        entry_flags: EntryFlags,
    ) {
        unsafe {
            self.mapper
                .identity_map_range(start_frame, end_frame, entry_flags, self.allocator)
        };
    }

    fn map_range(&mut self, start_page: Page, end_page: Page, flags: EntryFlags) {
        self.mapper
            .map_range(start_page, end_page, flags, self.allocator);
    }

    unsafe fn identity_map(&mut self, frame: Frame, flags: EntryFlags) {
        unsafe {
            self.mapper.identity_map(frame, flags, self.allocator);
        }
    }

    unsafe fn unmap_addr(&mut self, page: Page) -> Frame {
        unsafe { self.mapper.unmap_addr(page) }
    }

    unsafe fn map_to_range(
        &mut self,
        start_page: Page,
        end_page: Page,
        start_frame: Frame,
        end_frame: Frame,
        flags: EntryFlags,
    ) {
        unsafe {
            self.mapper.map_to_range(
                start_page,
                end_page,
                start_frame,
                end_frame,
                flags,
                self.allocator,
            );
        }
    }
}
