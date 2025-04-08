use core::{marker::PhantomData, ptr};

use bootbridge::{MemoryDescriptor, MemoryMap, MemoryType};

use crate::{
    direct_mapping,
    memory::{Frame, FrameAllocator, MAX_ALIGN, PAGE_SIZE},
    utils::NumberUtils,
};

pub struct BuddyAllocator<'a, const ORDER: usize> {
    free_lists: [FreeList; ORDER],
    max_mem: usize,
    allocated: usize,
    areas: Option<&'a MemoryMap<'a>>,
}

impl<'a, const ORDER: usize> FrameAllocator for BuddyAllocator<'a, ORDER> {
    fn allocate_frame(&mut self) -> Option<Frame> {
        return Some(Frame::containing_address(
            self.allocate(PAGE_SIZE as usize)? as u64,
        ));
    }

    fn deallocate_frame(&mut self, frame: Frame) {
        self.dealloc(
            frame.start_address().as_u64() as *mut u8,
            PAGE_SIZE as usize,
        );
    }
}

impl<'a, const ORDER: usize> BuddyAllocator<'a, ORDER> {
    pub unsafe fn new(areas: &'a MemoryMap<'a>) -> Self {
        let mut init = Self {
            free_lists: [const { FreeList::new() }; ORDER],
            max_mem: 0,
            allocated: 0,
            areas: Some(areas),
        };

        init.add_memory_map_to_area();

        return init;
    }

    unsafe fn add_memory_map_to_area(&mut self) {
        let areas = match self.areas {
            Some(areas) => areas,
            None => return,
        };
        for area in areas.entries().filter(|e| {
            matches!(
                e.ty,
                MemoryType::CONVENTIONAL | MemoryType::BOOT_SERVICES_CODE
            )
        }) {
            if area.phys_start == 0 {
                continue;
            }
            self.add_area(
                area.phys_start as usize,
                area.page_count as usize * PAGE_SIZE as usize,
            );
        }
    }

    pub fn allocated(&self) -> usize {
        self.allocated
    }

    unsafe fn add_area(&mut self, mut start_addr: usize, mut size: usize) {
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

            let node = &mut *((start_addr + offset) as *mut usize);
            *node = 0;
            self.free_lists[order.trailing_zeros() as usize - 1].push(node);
            offset += order;
            self.max_mem += order;
            size -= order;
        }
    }

    pub fn allocate(&mut self, mut size: usize) -> Option<*mut u8> {
        if !size.is_power_of_two() {
            size = size.next_power_of_two();
        }
        direct_mapping!({
            let order = size.trailing_zeros() as usize;

            let mut current_order = order;

            let mut some_mem = false;

            for (i, node) in self.free_lists[order - 1..].iter_mut().enumerate() {
                current_order = i + order;
                match node.is_empty() {
                    false => {
                        if current_order == order {
                            self.allocated += size;
                            return unsafe { node.pop().map(|e| e as *mut u8) };
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
        });
    }

    pub fn max_mem(&self) -> usize {
        self.max_mem
    }

    pub fn dealloc(&mut self, ptr: *mut u8, size: usize) {
        direct_mapping!({
            let mut order = size.trailing_zeros() as usize;
            let mut ptr = ptr as usize;

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
        });
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
        unsafe {
            *(self.prev) = *(self.curr);
        }
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
        *item = self.head as usize;
        self.head = item;
    }

    unsafe fn pop(&mut self) -> Option<*mut usize> {
        match self.is_empty() {
            true => None,
            false => {
                let item = self.head;
                self.head = *item as *mut usize;
                Some(item)
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.head.is_null()
    }

    fn iter_mut(&mut self) -> FreeListIterMut {
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

unsafe impl<const ORDER: usize> Send for BuddyAllocator<'_, ORDER> {}
