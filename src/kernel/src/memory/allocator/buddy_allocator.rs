use core::{marker::PhantomData, ptr};

use bootbridge::{MemoryMap, MemoryType};
use santa::Elf;
use x86_64::{instructions::tlb, VirtAddr};

use crate::{
    dwarf_data, log,
    memory::{
        paging::{
            create_mappings, table::RecurseLevel4, ActivePageTable, EntryFlags, InactivePageTable,
            Page,
        },
        Frame, FrameAllocator, MAX_ALIGN, PAGE_SIZE,
    },
    serial_println,
    utils::NumberUtils,
};

use super::linear_allocator::LinearAllocator;

struct AllocationContext {
    linear_allocator: LinearAllocator,
    map_access: Option<InactivePageTable>,
}

pub struct BuddyAllocator<'a, const ORDER: usize> {
    free_lists: [FreeList; ORDER],
    max_mem: usize,
    allocated: usize,
    areas: Option<&'a MemoryMap<'a>>,
    allocation_context: AllocationContext,
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
    pub unsafe fn new(
        memory_map: &'a MemoryMap<'a>,
        elf: &Elf<'a>,
        mut allocator: LinearAllocator,
    ) -> Self {
        let map_access = Some(create_mappings(
            |mapper, allocator| {
                elf.map_self(|start, end, flags| {
                    let start_frame = Frame::containing_address(start);
                    let end_frame = Frame::containing_address(end);

                    for frame in Frame::range_inclusive(start_frame, end_frame) {
                        mapper.identity_map(
                            frame,
                            EntryFlags::from_elf_program_flags(&flags),
                            allocator,
                        );
                    }
                });
                dwarf_data().map_self(|start, size| {
                    let start_frame = Frame::containing_address(start);
                    let end_frame = Frame::containing_address(start + size - 1);
                    for frame in Frame::range_inclusive(start_frame, end_frame) {
                        mapper.identity_map(
                            frame,
                            EntryFlags::PRESENT | EntryFlags::OVERWRITEABLE,
                            allocator,
                        );
                    }
                });
                mapper.identity_map_range(
                    (allocator.original_start() as u64).into(),
                    Frame::containing_address(
                        allocator.original_start() as u64 + allocator.size() as u64 - 1,
                    ),
                    EntryFlags::PRESENT | EntryFlags::WRITABLE,
                    allocator,
                );
            },
            &mut allocator,
        ));
        let mut init = Self {
            free_lists: [const { FreeList::new() }; ORDER],
            max_mem: 0,
            allocated: 0,
            areas: Some(memory_map),
            allocation_context: AllocationContext {
                linear_allocator: allocator,
                map_access,
            },
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
            if area.phys_start == 0
                || area.phys_start
                    == self.allocation_context.linear_allocator.original_start() as u64
            {
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
            direct_access(
                start_addr as u64 + offset as u64,
                &mut self.allocation_context,
                || {
                    *node = 0;
                },
            );
            self.free_lists[order.trailing_zeros() as usize - 1]
                .push(node, &mut self.allocation_context);
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
    //log!(Trace, "Buddy allocator is accessing: {address:#016x}");
    let mut active_table = unsafe { ActivePageTable::<RecurseLevel4>::new() };
    // Switch to the mapping
    let current_table = active_table.switch(ctx.map_access.take().unwrap());
    active_table.identity_map(
        Frame::containing_address(address),
        EntryFlags::WRITABLE,
        &mut ctx.linear_allocator,
    );
    if active_table.translate(VirtAddr::new(address)).is_none() {
        panic!("Failed to map address to the buddy allocator");
    }
    let result = f();
    active_table.unmap_addr(Page::containing_address(address));
    // Switch back
    let inactive_page_table = active_table.switch(current_table);
    ctx.map_access = Some(inactive_page_table);
    result
}

impl FreeList {
    const unsafe fn new() -> FreeList {
        FreeList {
            head: ptr::null_mut(),
        }
    }

    unsafe fn push(&mut self, item: *mut usize, ctx: &mut AllocationContext) {
        direct_access(item as u64, ctx, || {
            *item = self.head as usize;
        });
        self.head = item;
    }

    unsafe fn pop(&mut self, ctx: &mut AllocationContext) -> Option<*mut usize> {
        match self.is_empty() {
            true => None,
            false => {
                let item = self.head;
                direct_access(item as u64, ctx, || {
                    self.head = *item as *mut usize;
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

unsafe impl<const ORDER: usize> Send for BuddyAllocator<'_, ORDER> {}
