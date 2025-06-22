use core::ptr::write_bytes;

use alloc::vec;
use pager::{
    address::{Frame, PhysAddr, VirtAddr},
    allocator::{linear_allocator::LinearAllocator, FrameAllocator},
    page_table_size,
    paging::{
        table::{DirectLevel4, Table},
        ActivePageTable, Entry,
    },
    EntryFlags, Mapper, PageLevel, KERNEL_DIRECT_PHYSICAL_MAP, KERNEL_START, PAGE_SIZE,
};
use uefi::{
    proto::loaded_image::LoadedImage,
    table::boot::{AllocateType, MemoryDescriptor, MemoryMap, MemoryType},
};
use uefi_services::system_table;

use crate::context::{InitializationContext, Stage2, Stage3, Stage5, Stage6};

pub fn finialize_mapping(
    mut ctx: InitializationContext<Stage5>,
    memory_map: MemoryMap<'static>,
) -> InitializationContext<Stage6> {
    prepare_direct_map(&mut ctx, &memory_map);

    let mut kernel_table = ctx.active_table();

    let entry_size = ctx.context().entry_size;
    let allocator = &mut ctx.context_mut().allocator;

    let memory_map = bootbridge::MemoryMap::new(
        extract_memory_map(entry_size, memory_map),
        entry_size,
        MemoryDescriptor::VERSION as usize,
    );

    kernel_table.identity_map_object(&memory_map, allocator);

    ctx.next(memory_map)
}

fn extract_memory_map<'a>(
    entry_size: usize,
    memory_map: uefi::table::boot::MemoryMap<'a>,
) -> &'a [u8] {
    let entries = memory_map.entries();
    let start = memory_map.get(0).unwrap() as *const MemoryDescriptor as *const u8;
    let len = entries.len() * entry_size;
    unsafe { core::slice::from_raw_parts(start, len) }
}

fn prepare_direct_map(ctx: &mut InitializationContext<Stage5>, memory_map: &MemoryMap<'static>) {
    let mut kernel_table = ctx.active_table();
    let allocator = &mut ctx.context_mut().allocator;
    for usable in memory_map
        .entries()
        .filter(|e| e.ty == MemoryType::CONVENTIONAL)
    {
        let size = (usable.page_count * PAGE_SIZE) as usize;
        unsafe {
            kernel_table
                .mapper_with_allocator(allocator)
                .map_to_range_by_size(
                    VirtAddr::new(KERNEL_DIRECT_PHYSICAL_MAP.as_u64() + usable.phys_start).into(),
                    PhysAddr::new(usable.phys_start).into(),
                    size,
                    EntryFlags::WRITABLE,
                )
        };
    }
}

pub fn prepare_kernel_page(ctx: InitializationContext<Stage2>) -> InitializationContext<Stage3> {
    let system_table = system_table();
    let config = ctx.config();

    let mut buf = vec![0; system_table.boot_services().memory_map_size().map_size * 2];
    let mem_map = system_table
        .boot_services()
        .memory_map(&mut buf)
        .expect("FAILED TO GET MEMORY MAP");

    let mem_map_size = mem_map
        .entries()
        .filter(|e| {
            e.ty == MemoryType::CONVENTIONAL
                || e.ty == MemoryType::BOOT_SERVICES_DATA
                || e.ty == MemoryType::BOOT_SERVICES_CODE
                || e.ty == MemoryType::LOADER_DATA
        })
        .map(|e| (e.page_count * PAGE_SIZE) as usize)
        .sum::<usize>();
    let mem_map_size = page_table_size(mem_map_size, PageLevel::Page4K)
        + config.early_boot_kernel_page_table_byte_count();
    let mem_map_pages = (mem_map_size / PAGE_SIZE as usize) + 1;

    let kernel_pages_table = system_table
        .boot_services()
        .allocate_pages(
            AllocateType::AnyPages,
            MemoryType::LOADER_DATA,
            mem_map_pages,
        )
        .expect("Failed to allocate pages for kernel early page tables");

    unsafe {
        write_bytes(kernel_pages_table as *mut u8, 0, mem_map_size);
    }

    let mut kernel_page_allocator =
        unsafe { LinearAllocator::new(PhysAddr::new(kernel_pages_table), mem_map_size as usize) };

    let p4_frame = kernel_page_allocator.allocate_frame().unwrap();

    let mut kernel_table = unsafe {
        ActivePageTable::new_custom(p4_frame.start_address().as_u64() as *mut Table<DirectLevel4>)
    };

    kernel_table.identity_map_object(
        &kernel_page_allocator.mappings(),
        &mut kernel_page_allocator,
    );

    kernel_table.virtually_map_object(
        ctx.context().elf(),
        KERNEL_START,
        ctx.context().kernel_base,
        &mut kernel_page_allocator,
    );

    let protocol = system_table
        .boot_services()
        .open_protocol_exclusive::<LoadedImage>(system_table.boot_services().image_handle())
        .expect("Failed to open protocol for loaded image");

    let loaded_image_protocol = protocol.get().expect("Failed to get loaded image protocol");

    let (start, size) = loaded_image_protocol.info();

    unsafe {
        kernel_table
            .mapper_with_allocator(&mut kernel_page_allocator)
            .identity_map_by_size(
                Frame::containing_address(PhysAddr::new(start as u64)),
                size as usize,
                EntryFlags::PRESENT,
            );
    };

    // Do a recursive map
    kernel_table.p4_mut()[511] = Entry(
        p4_frame.start_address().as_u64() | (EntryFlags::PRESENT | EntryFlags::WRITABLE).bits(),
    );

    system_table
        .boot_services()
        .allocate_pages(
            AllocateType::Address(0x100000), // Ap bootstrap page table at 0x100000
            MemoryType::LOADER_DATA,
            64,
        ) // 64 page should be sufficent for kernel elf
        .expect("Failed to allocate pages for kernel ap tables");

    ctx.next((kernel_pages_table, kernel_page_allocator))
}
