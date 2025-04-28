use bootbridge::MemoryType;
use pager::{
    address::{PhysAddr, VirtAddr},
    allocator::{linear_allocator::LinearAllocator, FrameAllocator},
    paging::{
        create_mappings,
        table::{DirectLevel4, RecurseLevel4, Table},
        ActivePageTable, Entry,
    },
    registers::{Cr3, Cr3Flags},
    EntryFlags, Mapper, KERNEL_DIRECT_PHYSICAL_MAP, PAGE_SIZE,
};

use crate::{
    initialization_context::{InitializationContext, Phase0},
    DWARF_DATA,
};

use super::GENERAL_VIRTUAL_ALLOCATOR;

pub unsafe fn remap_the_kernel<A>(
    allocator: &mut A,
    ctx: &mut InitializationContext<Phase0>,
) -> ActivePageTable<RecurseLevel4>
where
    A: FrameAllocator,
{
    let new_table = create_mappings(
        |mapper, allocator| {
            mapper.virtually_map_object(
                ctx.context().boot_bridge().kernel_elf(),
                ctx.context().boot_bridge().kernel_base(),
                allocator,
            );

            for usable in ctx
                .context()
                .boot_bridge()
                .memory_map()
                .entries()
                .filter(|e| e.ty == MemoryType::CONVENTIONAL)
            {
                let size = (usable.page_count * PAGE_SIZE) as usize;
                unsafe {
                    mapper
                        .mapper_with_allocator(allocator)
                        .map_to_range_by_size(
                            VirtAddr::new(
                                KERNEL_DIRECT_PHYSICAL_MAP.as_u64() + usable.phys_start.as_u64(),
                            )
                            .into(),
                            usable.phys_start.into(),
                            size,
                            EntryFlags::WRITABLE,
                        )
                };
            }

            mapper.virtually_replace(
                &mut ctx.context_mut().boot_bridge,
                allocator,
                &GENERAL_VIRTUAL_ALLOCATOR,
            );
        },
        allocator,
    );

    let mut active_table = unsafe { ActivePageTable::<RecurseLevel4>::new() };
    unsafe { active_table.switch(new_table) };

    active_table
}
