use x86_64::{PhysAddr, VirtAddr};

use super::table::{
    DirectP4Create, HierarchicalLevel, NextTableAddress,
    RecurseP4Create, Table, TableLevel, TableLevel4,
};
use super::{EntryFlags, Page, ENTRY_COUNT};
use crate::memory::{Frame, FrameAllocator, PAGE_SIZE};
use core::ptr::Unique;

pub struct Mapper<P4: TableLevel4> {
    p4: Unique<Table<P4>>,
}

impl<P4> Mapper<P4>
where
    P4: TableLevel4,
    P4::CreateMarker: RecurseP4Create<P4>,
{
    pub unsafe fn new() -> Mapper<P4> {
        Mapper {
            p4: P4::CreateMarker::create(),
        }
    }
}

impl<P4> Mapper<P4>
where
    P4: TableLevel4,
    P4::CreateMarker: DirectP4Create<P4>,
{
    pub unsafe fn new_custom(p4: *mut Table<P4>) -> Mapper<P4> {
        Mapper {
            p4: P4::CreateMarker::create(p4),
        }
    }
}

/// Zero-cost ahhh abstraction
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
        unsafe { self.p4.as_ref() }
    }

    pub fn p4_mut(&mut self) -> &mut Table<P4> {
        unsafe { self.p4.as_mut() }
    }

    pub fn translate(&self, virtual_address: VirtAddr) -> Option<PhysAddr> {
        let offset = virtual_address.as_u64() % PAGE_SIZE;
        return self
            .translate_page(Page::containing_address(virtual_address.as_u64()))
            .map(|frame| PhysAddr::new(frame.number * PAGE_SIZE + offset));
    }

    pub fn translate_page(&self, page: Page) -> Option<Frame> {
        let p3 = self.p4().next_table(page.p4_index());

        let huge_page = || {
            p3.and_then(|p3| {
                let p3_entry = &p3[page.p3_index() as usize];
                // 1GiB page?
                if let Some(start_frame) = p3_entry.pointed_frame() {
                    if p3_entry.flags().contains(EntryFlags::HUGE_PAGE) {
                        // address must be 1GiB aligned
                        assert!(start_frame.number % (ENTRY_COUNT * ENTRY_COUNT) == 0);
                        return Some(Frame {
                            number: start_frame.number
                                + page.p2_index() * ENTRY_COUNT
                                + page.p1_index(),
                        });
                    }
                }
                if let Some(p2) = p3.next_table(page.p3_index()) {
                    let p2_entry = &p2[page.p2_index() as usize];
                    if let Some(start_frame) = p2_entry.pointed_frame() {
                        if p2_entry.flags().contains(EntryFlags::HUGE_PAGE) {
                            assert!(start_frame.number % ENTRY_COUNT == 0);
                            return Some(Frame {
                                number: start_frame.number + page.p1_index(),
                            });
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

    pub fn map_to<A>(&mut self, page: Page, frame: Frame, flags: EntryFlags, allocator: &mut A)
    where
        A: FrameAllocator,
    {
        let p4 = self.p4_mut();
        let p3 = p4.next_table_create(page.p4_index(), allocator);
        let p2 = p3.next_table_create(page.p3_index(), allocator);
        let p1 = p2.next_table_create(page.p2_index(), allocator);
        assert!(
            p1[page.p1_index() as usize].is_unused()
                || p1[page.p1_index() as usize].overwriteable()
        );
        p1[page.p1_index() as usize].set(frame, flags | EntryFlags::PRESENT);
    }
    pub fn map<A>(&mut self, page: Page, flags: EntryFlags, allocator: &mut A)
    where
        A: FrameAllocator,
    {
        let frame = allocator.allocate_frame().expect("out of memory");
        self.map_to(page, frame, flags, allocator)
    }

    pub fn identity_map<A>(&mut self, frame: Frame, flags: EntryFlags, allocator: &mut A)
    where
        A: FrameAllocator,
    {
        let page = Page::containing_address(frame.start_address().as_u64());
        self.map_to(page, frame, flags, allocator)
    }

    pub fn identity_map_range<A>(
        &mut self,
        start_frame: Frame,
        end_frame: Frame,
        flags: EntryFlags,
        allocator: &mut A,
    ) where
        A: FrameAllocator,
    {
        for frame in Frame::range_inclusive(start_frame, end_frame) {
            self.identity_map(frame, flags, allocator);
        }
    }

    pub fn unmap_addr(&mut self, page: Page) -> Frame {
        use x86_64::instructions::tlb;

        assert!(self
            .translate(VirtAddr::new(page.start_address()))
            .is_some());

        let p1 = self
            .p4_mut()
            .next_table_mut(page.p4_index())
            .and_then(|p3| p3.next_table_mut(page.p3_index()))
            .and_then(|p2| p2.next_table_mut(page.p2_index()))
            .expect("mapping code does not support huge pages");
        let frame = p1[page.p1_index() as usize].pointed_frame().unwrap();
        p1[page.p1_index() as usize].set_unused();
        tlb::flush(VirtAddr::new(page.start_address() as u64));
        frame
    }

    pub fn unmap<A>(&mut self, page: Page, allocator: &mut A)
    where
        A: FrameAllocator,
    {
        allocator.deallocate_frame(self.unmap_addr(page));
    }
}
