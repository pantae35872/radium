use bootbridge::MemoryType;
use pager::{
    EntryFlags, KERNEL_DIRECT_PHYSICAL_MAP,
    address::{Size4K, VirtAddr},
    allocator::FrameAllocator,
    paging::{
        ActivePageTable, InactivePageTable, TableManipulationContext,
        mapper::Mapper,
        table::{RootDirect, RootRecurse, Table},
        temporary_page::TemporaryTable,
    },
};

use crate::initialization_context::{InitializationContext, Stage0};

pub unsafe fn remap_the_kernel<A>(
    allocator: &mut A,
    ctx: &mut InitializationContext<Stage0>,
) -> ActivePageTable<RootRecurse>
where
    A: FrameAllocator,
{
    let mut active_table = unsafe { ActivePageTable::<RootRecurse>::new() };
    let mut temporary_page = TemporaryTable::new();

    let p4_frame = allocator.allocate_frame::<Size4K>().unwrap();

    for usable in ctx.context().boot_bridge().memory_map().entries().filter(|e| {
        matches!(
            e.ty,
            MemoryType::CONVENTIONAL
                | MemoryType::BOOT_SERVICES_DATA
                | MemoryType::BOOT_SERVICES_CODE
                | MemoryType::LOADER_DATA
                | MemoryType::LOADER_CODE
        )
    }) {
        if active_table.translate(usable.phys_start.assume_identity()).is_some() {
            continue;
        }
        let phys = usable.phys_start.into();
        unsafe { active_table.identity_map_auto(phys, usable.page_count as usize, EntryFlags::WRITABLE, allocator) };
    }

    let p4_table = p4_frame.start_address().as_u64() as *mut Table<RootDirect>;
    let mut new_table = unsafe { Mapper::new_custom(p4_table) };

    new_table.populate_p4_upper_half(allocator);

    for usable in ctx.context().boot_bridge().memory_map().entries().filter(|e| e.ty == MemoryType::CONVENTIONAL) {
        let virt = VirtAddr::new(KERNEL_DIRECT_PHYSICAL_MAP.as_u64() + usable.phys_start.as_u64()).into();
        let phys = usable.phys_start.into();
        unsafe { new_table.map_to_auto(virt, phys, usable.page_count as usize, EntryFlags::WRITABLE, allocator) };
    }

    new_table.transfer(&active_table, &mut ctx.context_mut().boot_bridge, allocator, true);
    new_table.p4_mut()[511].set(p4_frame, EntryFlags::PRESENT | EntryFlags::WRITABLE);

    let mut context =
        TableManipulationContext { temporary_page: &mut temporary_page, allocator, temporary_page_mapper: None };

    // SAFETY: The table is valid and not used right now
    let new_table = unsafe { InactivePageTable::from_raw(p4_frame) };

    unsafe { active_table.switch(&mut context, new_table) };

    active_table
}
