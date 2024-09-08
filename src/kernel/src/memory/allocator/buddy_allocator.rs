use core::ptr::NonNull;

use crate::utils::NumberUtils;

#[derive(Debug)]
struct FreeNode(Option<&'static mut FreeNode>);

struct FreeNoteIterMut<'a> {
    curr: &'a mut FreeNode,
}

#[derive(Debug)]
pub struct BuddyAllocator<const ORDER: usize> {
    free_lists: [FreeNode; ORDER],
    max_mem: usize,
    allocated: usize,
}

impl<'a> Iterator for FreeNoteIterMut<'a> {
    type Item = &'static mut FreeNode;

    fn next(&mut self) -> Option<Self::Item> {
        if self.curr.0.is_none() {
            return None;
        } else {
            unsafe {
                let curr = &mut *(self.curr as *mut FreeNode);
                self.curr = &mut *self.curr.as_ptr()?.as_ptr();
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

    fn as_ptr(&mut self) -> Option<NonNull<FreeNode>> {
        match &mut self.0 {
            Some(next) => NonNull::new(*next as *mut FreeNode),
            None => None,
        }
    }

    fn as_next_ptr(&mut self) -> Option<NonNull<u8>> {
        match &mut self.0 {
            Some(next) => NonNull::new(*next as *mut FreeNode as *mut u8),
            None => None,
        }
    }

    pub fn iter_mut(&mut self) -> FreeNoteIterMut {
        FreeNoteIterMut { curr: self }
    }
}

impl<const ORDER: usize> BuddyAllocator<ORDER> {
    pub unsafe fn new(start_addr: usize, mut max_mem: usize) -> Self {
        if !max_mem.is_power_of_two() {
            max_mem = max_mem.prev_power_of_two();
        }

        let mut init = Self {
            free_lists: [const { FreeNode(None) }; ORDER],
            max_mem,
            allocated: 0,
        };

        core::ptr::write_bytes(start_addr as *mut u8, 0, max_mem);
        let node = &mut *(start_addr as *mut FreeNode);
        init.free_lists[max_mem.trailing_zeros() as usize - 1] = FreeNode(Some(node));

        return init;
    }

    pub fn allocate(&mut self, size: usize) -> Option<NonNull<u8>> {
        assert!(size.is_power_of_two());

        if self.allocated >= self.max_mem {
            return None;
        }

        let order = size.trailing_zeros() as usize;

        let mut current_order = order;

        for (i, node) in self.free_lists[order - 1..].iter_mut().enumerate() {
            current_order = i + order;
            match node.0 {
                Some(_) => {
                    if current_order == order {
                        self.allocated += size;
                        return node.pop()?.as_next_ptr();
                    } else {
                        break;
                    }
                }
                None => continue,
            }
        }

        for i in (order..current_order).rev() {
            let (next_node, current_node) = {
                let (left, right) = self.free_lists.split_at_mut(i);
                (&mut left[i - 1], &mut right[0])
            };
            match current_node.pop() {
                Some(mut o_node) => {
                    let ptr = o_node.as_next_ptr().unwrap().as_ptr();

                    unsafe {
                        next_node.push(&mut *(ptr as *mut FreeNode));
                        next_node.push(&mut *((ptr as usize + (1 << i)) as *mut FreeNode));
                    }
                }
                None => continue,
            }
        }

        return self.allocate(size);
    }

    pub fn dealloc(&mut self, ptr: NonNull<u8>, size: usize) {
        let mut order = size.trailing_zeros() as usize;
        let mut ptr = ptr.as_ptr() as usize;

        self.free_lists[order - 1].push(unsafe { &mut *(ptr as *mut FreeNode) });

        while order <= ORDER {
            let buddy = ptr ^ (1 << order);
            let mut found_buddy = false;

            for block in self.free_lists[order - 1].iter_mut() {
                if block.as_ptr().is_some_and(|e| e.as_ptr() as usize == buddy) {
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
    }
}
