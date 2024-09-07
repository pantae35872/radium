use core::ptr::NonNull;

#[derive(Debug)]
#[repr(u64)]
enum FreeNode {
    Next(&'static mut FreeNode),
    None,
}

#[derive(Debug)]
pub struct BuddyAllocator<const ORDER: usize> {
    free_lists: [FreeNode; ORDER],
}

impl FreeNode {
    fn push(&mut self, new_node: &'static mut FreeNode) {
        let mut current = self;

        while let FreeNode::Next(ref mut next) = current {
            current = next;
        }

        *current = FreeNode::Next(new_node);
    }

    fn pop(&mut self) -> Option<FreeNode> {
        match self {
            FreeNode::Next(_) => {
                let mut removed = core::mem::replace(self, FreeNode::None);

                if let FreeNode::Next(ref mut next) = removed {
                    if let FreeNode::Next(ref mut next) = next {
                        *self = FreeNode::Next(unsafe { &mut *(*next as *mut FreeNode) });
                    }
                }
                return Some(removed);
            }
            FreeNode::None => None,
        }
    }

    fn as_next_ptr(&mut self) -> Option<NonNull<u8>> {
        match self {
            Self::Next(next) => NonNull::new(*next as *mut FreeNode as *mut u8),
            Self::None => None,
        }
    }
}

impl<const ORDER: usize> BuddyAllocator<ORDER> {
    pub unsafe fn new(start_addr: usize, size: usize) -> Self {
        assert!(size.is_power_of_two());

        let mut init = Self {
            free_lists: [const { FreeNode::None }; ORDER],
        };

        core::ptr::write_bytes(start_addr as *mut u8, 0, size);
        let node = &mut *(start_addr as *mut FreeNode);
        *node = FreeNode::None;
        init.free_lists[size.trailing_zeros() as usize - 1] = FreeNode::Next(node);

        return init;
    }

    pub fn allocate(&mut self, size: usize) -> Option<NonNull<u8>> {
        assert!(size.is_power_of_two());
        let order = size.trailing_zeros() as usize;

        let mut current_order = order;

        for (i, node) in self.free_lists[order - 1..].iter_mut().enumerate() {
            current_order = i + order;
            match node {
                FreeNode::Next(_) => {
                    if current_order == order {
                        return node.pop()?.as_next_ptr();
                    } else {
                        break;
                    }
                }
                FreeNode::None => continue,
            }
        }

        for i in (order..current_order).rev() {
            let (node_0, node_1) = {
                let (left, right) = self.free_lists.split_at_mut(i);
                (&mut left[i - 1], &mut right[0])
            };
            match node_1.pop() {
                Some(mut o_node) => {
                    let ptr = o_node.as_next_ptr().unwrap().as_ptr();

                    unsafe {
                        core::ptr::write_bytes(ptr, 0, size_of::<FreeNode>());
                        let end = &mut *((ptr as usize + (1 << i)) as *mut FreeNode);
                        let start = &mut *(ptr as *mut FreeNode);
                        *end = FreeNode::None;
                        *start = FreeNode::None;
                        node_0.push(start);
                        node_0.push(end);
                    }
                }
                None => continue,
            }
        }

        return self.allocate(size);
    }

    pub fn dealloc(&mut self, _ptr: NonNull<u8>, _size: usize) {
        todo!()
    }
}
