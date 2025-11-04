use bootbridge::MemoryType;
use pager::{
    EntryFlags, KERNEL_DIRECT_PHYSICAL_MAP, KERNEL_START, Mapper, PAGE_SIZE,
    address::{Page, VirtAddr},
    allocator::FrameAllocator,
    paging::{
        ActivePageTable, TableManipulationContext, create_mappings, table::RecurseLevel4,
        temporary_page::TemporaryPage,
    },
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
    let mut active_table = unsafe { ActivePageTable::<RecurseLevel4>::new() };

    let mut temporary_page = TemporaryPage::new(Page::deadbeef());
    let mut context = TableManipulationContext {
        temporary_page: &mut temporary_page,
        allocator,
    };

    let new_table = create_mappings(
        |mapper, allocator| {
            unsafe {
                ctx.context().boot_bridge().kernel_elf().map_permission(
                    &mut mapper.mapper_with_allocator(allocator),
                    KERNEL_START,
                    ctx.context().boot_bridge().kernel_base(),
                )
            };

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
        &mut context,
        &mut active_table,
    );

    unsafe { active_table.switch(&mut context, new_table) };

    active_table
}
