use uefi::table::boot::{MemoryMap, MemoryType};

use crate::{
    direct_mapping,
    memory::{Frame, FrameAllocator, PAGE_SIZE},
    utils::NumberUtils,
};

#[derive(Debug)]
struct FreeNode(Option<&'static mut FreeNode>);

struct FreeNoteIterMut<'a> {
    curr: &'a mut FreeNode,
}

pub struct BuddyAllocator<'a, const ORDER: usize> {
    free_lists: [FreeNode; ORDER],
    max_mem: usize,
    allocated: usize,
    areas: Option<&'a MemoryMap<'a>>,
    current_range: usize,
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
    pub unsafe fn addr_new(start_addr: usize, mut max_mem: usize) -> Self {
        if !max_mem.is_power_of_two() {
            max_mem = max_mem.prev_power_of_two();
        }

        let mut init = Self {
            free_lists: [const { FreeNode(None) }; ORDER],
            max_mem,
            allocated: 0,
            areas: None,
            current_range: 0,
        };

        core::ptr::write_bytes(start_addr as *mut u8, 0, max_mem);
        let node = &mut *(start_addr as *mut FreeNode);
        init.free_lists[max_mem.trailing_zeros() as usize - 1] = FreeNode(Some(node));

        return init;
    }
    pub unsafe fn new(areas: &'a MemoryMap<'a>) -> Self {
        let mut init = Self {
            free_lists: [const { FreeNode(None) }; ORDER],
            max_mem: 0,
            allocated: 0,
            current_range: 0,
            areas: Some(areas),
        };

        init.current_range += 1;
        init.select_next_area();

        return init;
    }

    fn select_next_area(&mut self) {
        if let Some(areas) = self.areas {
            let area = areas
                .entries()
                .filter(|e| e.ty == MemoryType::CONVENTIONAL)
                .nth(self.current_range)
                .expect("Out of memory");
            self.current_range += 1;
            unsafe {
                self.add_area(
                    area.phys_start as usize,
                    area.page_count as usize * PAGE_SIZE as usize,
                );
            }
        }
    }

    //TODO: Add dynamic sizeing
    unsafe fn add_area(&mut self, start_addr: usize, mut size: usize) {
        if !size.is_power_of_two() {
            size = size.prev_power_of_two();
        }
        //if !(start_addr as *mut u8).is_aligned_to(size) {
        //    let prev_addr = start_addr;
        //    start_addr = (start_addr as *mut u8).align_offset(align);
        //    size -= start_addr - prev_addr;
        //}

        let node = &mut *(start_addr as *mut FreeNode);
        self.free_lists[size.trailing_zeros() as usize - 1].push(node);
        self.max_mem += size;
    }

    pub fn allocate(&mut self, size: usize) -> Option<*mut u8> {
        direct_mapping!({
            let order = size.trailing_zeros() as usize;

            let mut current_order = order;

            let mut some_mem = false;

            for (i, node) in self.free_lists[order - 1..].iter_mut().enumerate() {
                current_order = i + order;
                match node.0 {
                    Some(_) => {
                        if current_order == order {
                            self.allocated += size;
                            return node.pop()?.as_next_ptr();
                        } else {
                            some_mem = true;
                            break;
                        }
                    }
                    None => continue,
                }
            }

            if some_mem {
                for i in (order..current_order).rev() {
                    let (next_node, current_node) = {
                        let (left, right) = self.free_lists.split_at_mut(i);
                        (&mut left[i - 1], &mut right[0])
                    };
                    match current_node.pop() {
                        Some(mut o_node) => {
                            let ptr = o_node.as_next_ptr().unwrap();

                            unsafe {
                                next_node.push(&mut *(ptr as *mut FreeNode));
                                next_node.push(&mut *((ptr as usize + (1 << i)) as *mut FreeNode));
                            }
                        }
                        None => continue,
                    }
                }
                return self.allocate(size);
            } else {
                self.select_next_area();
                if self.allocated + size >= self.max_mem {
                    return None;
                }
                return self.allocate(size);
            }
        });
    }

    pub fn dealloc(&mut self, ptr: *mut u8, size: usize) {
        direct_mapping!({
            let mut order = size.trailing_zeros() as usize;
            let mut ptr = ptr as usize;

            self.free_lists[order - 1].push(unsafe { &mut *(ptr as *mut FreeNode) });

            while order <= ORDER {
                let buddy = ptr ^ (1 << order);
                let mut found_buddy = false;

                for block in self.free_lists[order - 1].iter_mut() {
                    if block.as_ptr().is_some_and(|e| e as usize == buddy) {
                        block.pop();
                        found_buddy = true;
                        break;
                    }
                }

                if found_buddy {
                    self.free_lists[order - 1].pop();
                    ptr = ptr.min(buddy);
                    order += 1;
                    self.free_lists[order - 1].push(unsafe { &mut *(ptr as *mut FreeNode) });
                } else {
                    break;
                }
            }

            self.allocated -= size;
        });
    }
}

impl<'a> Iterator for FreeNoteIterMut<'a> {
    type Item = &'static mut FreeNode;

    fn next(&mut self) -> Option<Self::Item> {
        if self.curr.0.is_none() {
            return None;
        } else {
            unsafe {
                let curr = &mut *(self.curr as *mut FreeNode);
                self.curr = &mut *self.curr.as_ptr()?;
                return Some(curr);
            }
        }
    }
}

impl FreeNode {
    fn push(&mut self, new_node: &'static mut FreeNode) {
        let mut current = self;

        while let Some(ref mut next) = current.0 {
            current = next;
        }

        *new_node = FreeNode(None);
        *current = FreeNode(Some(new_node));
    }

    fn pop(&mut self) -> Option<FreeNode> {
        match self.0 {
            Some(_) => {
                let mut removed = core::mem::replace(self, FreeNode(None));

                if let Some(ref mut next) = removed.0 {
                    if let Some(ref mut next) = next.0 {
                        *self = FreeNode(Some(unsafe { &mut *(*next as *mut FreeNode) }));
                    }
                }
                return Some(removed);
            }
            None => None,
        }
    }

    fn as_ptr(&mut self) -> Option<*mut FreeNode> {
        match &mut self.0 {
            Some(next) => Some(*next as *mut FreeNode),
            None => None,
        }
    }

    fn as_next_ptr(&mut self) -> Option<*mut u8> {
        match &mut self.0 {
            Some(next) => Some(*next as *mut FreeNode as *mut u8),
            None => None,
        }
    }

    pub fn iter_mut(&mut self) -> FreeNoteIterMut {
        FreeNoteIterMut { curr: self }
    }
}
