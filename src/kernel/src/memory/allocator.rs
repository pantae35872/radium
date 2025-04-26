use core::ptr;

use crate::{
    initialization_context::{InitializationContext, Phase1},
    memory::virt_addr_alloc,
};
use alloc::alloc::*;
use pager::{
    address::{Page, VirtAddr},
    EntryFlags, Mapper, PAGE_SIZE,
};

pub mod area_allocator;
pub mod buddy_allocator;
pub mod linked_list;

use self::linked_list::LinkedListAllocator;

pub const HEAP_SIZE: u64 = 0x4000000; // 64 Mib

pub fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

pub struct Locked<A> {
    inner: spin::Mutex<A>,
}

impl<A> Locked<A> {
    pub const fn new(inner: A) -> Self {
        Locked {
            inner: spin::Mutex::new(inner),
        }
    }

    pub fn lock(&self) -> spin::MutexGuard<A> {
        self.inner.lock()
    }
}

unsafe impl GlobalAlloc for Locked<LinkedListAllocator> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let (size, align) = LinkedListAllocator::size_align(layout);
        let mut allocator = self.lock();

        if let Some((region, alloc_start)) = allocator.find_region(size, align) {
            let alloc_end = alloc_start.checked_add(size).expect("overflow");
            let excess_size = region.end_addr() - alloc_end;
            if excess_size > 0 {
                unsafe { allocator.add_free_region(alloc_end, excess_size) };
            }
            alloc_start as *mut u8
        } else {
            ptr::null_mut()
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let (size, _) = LinkedListAllocator::size_align(layout);
        unsafe { self.lock().add_free_region(ptr as usize, size) };
    }
}

#[global_allocator]
static GLOBAL_ALLOCATOR: Locked<LinkedListAllocator> = Locked::new(LinkedListAllocator::new());

/// Initialize the kernel heap
///
/// SAFETY:
/// The caller must ensure that this is called on kernel initializaton only
/// And must be called after the memory controller is initialize
pub unsafe fn init(ctx: &mut InitializationContext<Phase1>) {
    let heap_start = virt_addr_alloc(HEAP_SIZE / PAGE_SIZE);
    ctx.mapper().map_range(
        heap_start,
        Page::containing_address(VirtAddr::new(
            heap_start.start_address().as_u64() + HEAP_SIZE - 1,
        )),
        EntryFlags::WRITABLE,
    );
    unsafe {
        GLOBAL_ALLOCATOR.lock().init(
            heap_start.start_address().as_u64() as usize,
            HEAP_SIZE as usize,
        );
    }
}
