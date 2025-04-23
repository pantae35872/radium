use core::{marker::PhantomData, ptr};

use bootbridge::MemoryDescriptor;
use pager::address::{Page, PhysAddr, VirtAddr};
use pager::EntryFlags;

use crate::initialization_context::{InitializationContext, Phase0};
use crate::interrupt;
use crate::memory::stack_allocator::StackAllocator;
use crate::{
    dwarf_data,
    memory::{
        paging::{create_mappings, table::RecurseLevel4, ActivePageTable, InactivePageTable},
        Frame, FrameAllocator, MAX_ALIGN, PAGE_SIZE,
    },
    utils::NumberUtils,
};

use super::{area_allocator::AreaAllocator, linear_allocator::LinearAllocator};

struct AllocationContext {
    linear_allocator: LinearAllocator,
    access_map: Option<InactivePageTable>,
}

pub struct BuddyAllocator<const ORDER: usize> {
    free_lists: [FreeList; ORDER],
    max_mem: usize,
    allocated: usize,
    allocation_context: AllocationContext,
}

impl<const ORDER: usize> FrameAllocator for BuddyAllocator<ORDER> {
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
        mut allocator: LinearAllocator,
        area_allocator: AreaAllocator<'a, impl Iterator<Item = &'a MemoryDescriptor>>,
        stack_allocator: &StackAllocator,
        ctx: &InitializationContext<Phase0>,
    ) -> Self {
        let map_access = Some(create_mappings(
            |mapper, allocator| {
                mapper.identity_map_object(ctx.context().boot_bridge(), allocator);
                mapper.identity_map_object(stack_allocator, allocator);
                mapper.identity_map_object(dwarf_data(), allocator);
                mapper.identity_map_object(&allocator.mappings(), allocator);
                //mapper.identity_map_object(allocator, allocator);
            },
            &mut allocator,
        ));
        let mut init = Self {
            free_lists: [const { unsafe { FreeList::new() } }; ORDER],
            max_mem: 0,
            allocated: 0,
            allocation_context: AllocationContext {
                linear_allocator: allocator,
                access_map: map_access,
            },
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
        let mut start_addr = start_addr.as_u64() as usize;
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

            direct_access(
                start_addr as u64 + offset as u64,
                &mut self.allocation_context,
                || unsafe {
                    *((start_addr + offset) as *mut usize) = 0;
                },
            );

            unsafe {
                self.free_lists[order.trailing_zeros() as usize - 1].push(
                    (start_addr + offset) as *mut usize,
                    &mut self.allocation_context,
                )
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
                        let addr =
                            unsafe { node.pop(&mut self.allocation_context).map(|e| e as *mut u8) };
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
            match unsafe { current_node.pop(&mut self.allocation_context) } {
                Some(o_node) => unsafe {
                    next_node.push(o_node, &mut self.allocation_context);
                    next_node.push(
                        (o_node as usize + (1 << i)) as *mut usize,
                        &mut self.allocation_context,
                    );
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
        let mut ptr = ptr as usize;

        unsafe {
            self.free_lists[order - 1].push(ptr as *mut usize, &mut self.allocation_context);
        }

        while order <= ORDER {
            let buddy = ptr ^ (1 << order);
            let mut found_buddy = false;

            for block in self.free_lists[order - 1].iter_mut(&mut self.allocation_context) {
                if block.value() as usize == buddy {
                    block.pop(&mut self.allocation_context);
                    found_buddy = true;
                    break;
                }
            }

            if found_buddy {
                unsafe {
                    self.free_lists[order - 1].pop(&mut self.allocation_context);
                }
                ptr = ptr.min(buddy);
                order += 1;
                unsafe {
                    self.free_lists[order - 1]
                        .push(ptr as *mut usize, &mut self.allocation_context);
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
    fn pop(self, ctx: &mut AllocationContext) -> *mut usize {
        let addr = direct_access(self.curr as u64, ctx, || unsafe { *(self.curr) });
        direct_access(self.prev as u64, ctx, || unsafe { *(self.prev) = addr });
        self.curr
    }

    fn value(&self) -> *mut usize {
        self.curr
    }
}

fn direct_access<T>(address: u64, ctx: &mut AllocationContext, f: impl FnOnce() -> T) -> T {
    // Without interrupts because we didn't have the mappings for the device and apic
    interrupt::without_interrupts(|| {
        let mut active_table = unsafe { ActivePageTable::<RecurseLevel4>::new() };
        // SAFETY: This should be safe if the allocator table is correctly mapped
        let current_table = unsafe {
            let current_table = active_table.switch(ctx.access_map.take().unwrap());
            active_table.identity_map(
                Frame::containing_address(PhysAddr::new(address)),
                EntryFlags::WRITABLE,
                &mut ctx.linear_allocator,
            );
            current_table
        };
        let result = f();
        // SAFETY: We know that we called identity map above
        unsafe { active_table.unmap_addr(Page::containing_address(VirtAddr::new(address))) };
        // Switch back
        // SAFETY: We know that the table is valid beca
        let inactive_page_table = unsafe { active_table.switch(current_table) };
        ctx.access_map = Some(inactive_page_table);
        result
    })
}

impl FreeList {
    const unsafe fn new() -> FreeList {
        FreeList {
            head: ptr::null_mut(),
        }
    }

    unsafe fn push(&mut self, item: *mut usize, ctx: &mut AllocationContext) {
        direct_access(item as u64, ctx, || {
            unsafe { *item = self.head as usize };
        });
        self.head = item;
    }

    unsafe fn pop(&mut self, ctx: &mut AllocationContext) -> Option<*mut usize> {
        match self.is_empty() {
            true => None,
            false => {
                let item = self.head;
                direct_access(item as u64, ctx, || {
                    self.head = unsafe { *item as *mut usize };
                });
                Some(item)
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.head.is_null()
    }

    fn iter_mut<'a, 'c>(&'a mut self, ctx: &'c mut AllocationContext) -> FreeListIterMut<'a, 'c> {
        FreeListIterMut {
            list: PhantomData,
            previous: &mut self.head as *mut *mut usize as *mut usize,
            current: self.head,
            ctx,
        }
    }
}

struct FreeListIterMut<'a, 'c> {
    list: PhantomData<&'a mut FreeList>,
    previous: *mut usize,
    current: *mut usize,
    ctx: &'c mut AllocationContext,
}
impl<'a, 'c> Iterator for FreeListIterMut<'a, 'c> {
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
            direct_access(self.current as u64, self.ctx, || {
                self.current = unsafe { *self.current as *mut usize };
            });
            Some(res)
        }
    }
}

unsafe impl<const ORDER: usize> Send for BuddyAllocator<ORDER> {}
