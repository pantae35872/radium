use pager::{
    address::PhysAddr,
    allocator::{linear_allocator::LinearAllocator, FrameAllocator},
    paging::{
        create_mappings,
        table::{DirectLevel4, RecurseLevel4, Table},
        ActivePageTable, Entry,
    },
    registers::{Cr3, Cr3Flags},
    EntryFlags,
};

use crate::{
    dwarf_data,
    initialization_context::{InitializationContext, Phase0},
};

use super::stack_allocator::StackAllocator;

/// Initialize the first recursive map and the kernel stack
///
/// # Safety
/// The caller must ensure that this is only called on kernel initialization
pub unsafe fn early_map_kernel(
    ctx: &InitializationContext<Phase0>,
    buddy_allocator_allocator: &LinearAllocator,
) {
    unsafe extern "C" {
        pub static early_alloc: u8;
    }

    let mut allocator =
        unsafe { LinearAllocator::new(PhysAddr::new(&early_alloc as *const u8 as u64), 4096 * 64) };

    let p4_table = allocator
        .allocate_frame()
        .expect("Failed to allocate frame for temporary early boot");
    let mut active_table = unsafe {
        ActivePageTable::<DirectLevel4>::new_custom(
            p4_table.start_address().as_u64() as *mut Table<DirectLevel4>
        )
    };

    active_table.identity_map_object(ctx.context().boot_bridge(), &mut allocator);
    active_table.identity_map_object(dwarf_data(), &mut allocator);
    active_table.identity_map_object(&buddy_allocator_allocator.mappings(), &mut allocator);

    // Do a recursive map
    active_table.p4_mut()[511] = Entry(
        p4_table.start_address().as_u64() | (EntryFlags::PRESENT | EntryFlags::WRITABLE).bits(),
    );

    // Unsafely change the cr3 bc we have no recursive map on the uefi table
    // TODO: If we want to do this safely and by design, we need huge pages support for both L3 and
    // L2 huge pages bc we can't work with the uefi table without huge pages support
    unsafe {
        Cr3::write(
            p4_table.start_address().into(),
            Cr3Flags::PAGE_LEVEL_WRITETHROUGH,
        )
    };
}

pub unsafe fn remap_the_kernel<A>(
    allocator: &mut A,
    stack_allocator: &StackAllocator,
    bootbridge: &InitializationContext<Phase0>,
) -> ActivePageTable<RecurseLevel4>
where
    A: FrameAllocator,
{
    let new_table = create_mappings(
        |mapper, allocator| {
            mapper.identity_map_object(bootbridge.context().boot_bridge(), allocator);
            mapper.identity_map_object(stack_allocator, allocator);
            mapper.identity_map_object(dwarf_data(), allocator);
        },
        allocator,
    );

    let mut active_table = unsafe { ActivePageTable::<RecurseLevel4>::new() };
    unsafe { active_table.switch(new_table) };

    active_table
}
