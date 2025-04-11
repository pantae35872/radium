use core::ptr;

use crate::memory::virt_addr_alloc;
use alloc::alloc::*;
use lazy_static::lazy_static;

pub mod buddy_allocator;
pub mod linear_allocator;
pub mod linked_list;

use self::linked_list::LinkedListAllocator;

use super::memory_controller;

lazy_static! {
    pub static ref HEAP_START: u64 = virt_addr_alloc(HEAP_SIZE);
}
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
                allocator.add_free_region(alloc_end, excess_size);
            }
            alloc_start as *mut u8
        } else {
            ptr::null_mut()
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let (size, _) = LinkedListAllocator::size_align(layout);
        self.lock().add_free_region(ptr as usize, size);
    }
}

#[global_allocator]
static GLOBAL_ALLOCATOR: Locked<LinkedListAllocator> = Locked::new(LinkedListAllocator::new());

pub fn init() {
    memory_controller().lock().alloc_map(HEAP_SIZE, *HEAP_START);

    unsafe {
        GLOBAL_ALLOCATOR
            .lock()
            .init(*HEAP_START as usize, HEAP_SIZE as usize);
    }
}
