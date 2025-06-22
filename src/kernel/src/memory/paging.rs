use bootbridge::MemoryType;
use pager::{
    address::VirtAddr,
    allocator::FrameAllocator,
    paging::{create_mappings, table::RecurseLevel4, ActivePageTable},
    EntryFlags, Mapper, KERNEL_DIRECT_PHYSICAL_MAP, KERNEL_START, PAGE_SIZE,
};

use crate::initialization_context::{InitializationContext, Stage0};

use super::GENERAL_VIRTUAL_ALLOCATOR;

pub unsafe fn remap_the_kernel<A>(
    allocator: &mut A,
    ctx: &mut InitializationContext<Stage0>,
) -> ActivePageTable<RecurseLevel4>
where
    A: FrameAllocator,
{
    let new_table = create_mappings(
        |mapper, allocator| {
            mapper.virtually_map_object(
                ctx.context().boot_bridge().kernel_elf(),
                KERNEL_START,
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
