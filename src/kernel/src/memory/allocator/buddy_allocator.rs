use core::{marker::PhantomData, ptr};

use bootbridge::MemoryDescriptor;
use pager::KERNEL_DIRECT_PHYSICAL_MAP;
use pager::address::PhysAddr;
use pager::allocator::FrameAllocator;

use crate::{
    memory::{Frame, MAX_ALIGN, PAGE_SIZE},
    utils::NumberUtils,
};

use super::area_allocator::AreaAllocator;

pub struct BuddyAllocator<const ORDER: usize = 64> {
    free_lists: [FreeList; ORDER],
    max_mem: usize,
    allocated: usize,
}

// SAFETY: this is uphold by the implementation of the buddy allocator to be correct
unsafe impl<const ORDER: usize> FrameAllocator for BuddyAllocator<ORDER> {
    fn allocate_frame(&mut self) -> Option<Frame> {
        self.allocate(PAGE_SIZE as usize)
            .map(|e| Frame::containing_address(PhysAddr::new(e as u64)))
    }

    fn deallocate_frame(&mut self, frame: Frame) {
        self.dealloc(
            frame.start_address().as_u64() as *mut u8,
            PAGE_SIZE as usize,
        );
    }
}

impl<const ORDER: usize> BuddyAllocator<ORDER> {
    pub unsafe fn new<'a>(
        area_allocator: AreaAllocator<'a, impl Iterator<Item = &'a MemoryDescriptor>>,
    ) -> Self {
        let mut init = Self {
            free_lists: [const { unsafe { FreeList::new() } }; ORDER],
            max_mem: 0,
            allocated: 0,
        };

        unsafe { init.add_entire_memory_to_area(area_allocator) };

        return init;
    }

    unsafe fn add_entire_memory_to_area<'a>(
        &mut self,
        mut area_allocator: AreaAllocator<'a, impl Iterator<Item = &'a MemoryDescriptor>>,
    ) {
        while let Some((start, size)) = area_allocator.allocate_entire_buffer() {
            unsafe { self.add_area(start, size) };
        }
    }

    pub fn allocated(&self) -> usize {
        self.allocated
    }

    unsafe fn add_area(&mut self, start_addr: PhysAddr, mut size: usize) {
        let mut start_addr =
            KERNEL_DIRECT_PHYSICAL_MAP.as_u64() as usize + start_addr.as_u64() as usize;
        let unaligned_addr = start_addr;
        if !(start_addr as *const u8).is_aligned_to(MAX_ALIGN) {
            start_addr += (start_addr as *const u8).align_offset(MAX_ALIGN);
        }
        size -= start_addr - unaligned_addr;

        let mut offset = 0;
        while size > 0 {
            let order = size.prev_power_of_two();

            if order < 8 {
                break;
            }

            unsafe {
                *((start_addr + offset) as *mut usize) = 0;
            };

            unsafe {
                self.free_lists[order.trailing_zeros() as usize - 1]
                    .push((start_addr + offset) as *mut usize)
            };

            offset += order;
            self.max_mem += order;
            size -= order;
        }
    }

    pub fn allocate(&mut self, mut size: usize) -> Option<*mut u8> {
        if !size.is_power_of_two() {
            size = size.next_power_of_two();
        }
        let order = size.trailing_zeros() as usize;

        let mut current_order = order;

        let mut some_mem = false;

        for (i, node) in self.free_lists[order - 1..].iter_mut().enumerate() {
            current_order = i + order;
            match node.is_empty() {
                false => {
                    if current_order == order {
                        self.allocated += size;
                        let addr = unsafe {
                            node.pop().map(|e| {
                                (e as u64 - KERNEL_DIRECT_PHYSICAL_MAP.as_u64()) as *mut u8
                            })
                        };
                        return addr;
                    } else {
                        some_mem = true;
                        break;
                    }
                }
                true => continue,
            }
        }

        if !some_mem {
            return None;
        }

        for i in (order..current_order).rev() {
            let (next_node, current_node) = {
                let (left, right) = self.free_lists.split_at_mut(i);
                (&mut left[i - 1], &mut right[0])
            };
            match unsafe { current_node.pop() } {
                Some(o_node) => unsafe {
                    next_node.push(o_node);
                    next_node.push((o_node as usize + (1 << i)) as *mut usize);
                },
                None => continue,
            }
        }
        return self.allocate(size);
    }

    pub fn max_mem(&self) -> usize {
        self.max_mem
    }

    pub fn dealloc(&mut self, ptr: *mut u8, size: usize) {
        let mut order = size.trailing_zeros() as usize;
        let mut ptr = KERNEL_DIRECT_PHYSICAL_MAP.as_u64() as usize + ptr as usize;

        unsafe {
            self.free_lists[order - 1].push(ptr as *mut usize);
        }

        while order <= ORDER {
            let buddy = ptr ^ (1 << order);
            let mut found_buddy = false;

            for block in self.free_lists[order - 1].iter_mut() {
                if block.value() as usize == buddy {
                    block.pop();
                    found_buddy = true;
                    break;
                }
            }

            if found_buddy {
                unsafe {
                    self.free_lists[order - 1].pop();
                }
                ptr = ptr.min(buddy);
                order += 1;
                unsafe {
                    self.free_lists[order - 1].push(ptr as *mut usize);
                }
            } else {
                break;
            }
        }

        self.allocated -= size;
    }
}

/// Code below is taken from https://github.com/rcore-os/buddy_system_allocator/blob/master/src/linked_list.rs
#[derive(Debug)]
struct FreeNode {
    prev: *mut usize,
    curr: *mut usize,
}

struct FreeList {
    head: *mut usize,
}

impl FreeNode {
    fn pop(self) -> *mut usize {
        let addr = unsafe { *(self.curr) };
        unsafe { *(self.prev) = addr };
        self.curr
    }

    fn value(&self) -> *mut usize {
        self.curr
    }
}

impl FreeList {
    const unsafe fn new() -> FreeList {
        FreeList {
            head: ptr::null_mut(),
        }
    }

    unsafe fn push(&mut self, item: *mut usize) {
        unsafe { *item = self.head as usize };
        self.head = item;
    }

    unsafe fn pop(&mut self) -> Option<*mut usize> {
        match self.is_empty() {
            true => None,
            false => {
                let item = self.head;
                self.head = unsafe { *item as *mut usize };
                Some(item)
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.head.is_null()
    }

    fn iter_mut<'a>(&'a mut self) -> FreeListIterMut<'a> {
        FreeListIterMut {
            list: PhantomData,
            previous: &mut self.head as *mut *mut usize as *mut usize,
            current: self.head,
        }
    }
}

struct FreeListIterMut<'a> {
    list: PhantomData<&'a mut FreeList>,
    previous: *mut usize,
    current: *mut usize,
}
impl<'a> Iterator for FreeListIterMut<'a> {
    type Item = FreeNode;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current.is_null() {
            None
        } else {
            let res = FreeNode {
                prev: self.previous,
                curr: self.current,
            };
            self.previous = self.current;
            self.current = unsafe { *self.current as *mut usize };
            Some(res)
        }
    }
}

unsafe impl<const ORDER: usize> Send for BuddyAllocator<ORDER> {}
