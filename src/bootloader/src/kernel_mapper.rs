use bootbridge::{BootBridge, BootBridgeBuilder, RawBootBridge};
use pager::{
    EntryFlags,
    address::{PageSize, PhysAddr, Size4K},
    allocator::{FrameAllocator, linear_allocator::LinearAllocator},
    paging::{
        mapper::Mapper,
        table::{RootDirect, Table},
    },
    registers::Cr3,
};
use uefi::{
    proto::loaded_image::LoadedImage,
    table::boot::{AllocateType, MemoryDescriptor, MemoryMap, MemoryType},
};
use uefi_services::system_table;

use crate::{
    boot_services::BootServiceFrameAlloc,
    context::{InitializationContext, Stage2, Stage3, Stage5},
};

pub fn finialize_mapping(
    ctx: InitializationContext<Stage5>,
    mut builder: BootBridgeBuilder,
    memory_map: MemoryMap<'static>,
) -> *mut RawBootBridge {
    use bootbridge::MemoryMap;

    let entry_size = ctx.context().entry_size;

    let memory_map =
        MemoryMap::new(extract_memory_map(entry_size, memory_map), entry_size, MemoryDescriptor::VERSION as usize);
    let uefi_mapping =
        unsafe { Mapper::<RootDirect>::new_custom(Cr3::read().0.start_address().assume_identity().as_mut_ptr()) };

    let mut mapper = ctx.mapper();
    let mut allocator = ctx.context.temporary_runtime_allocator;

    builder
        .memory_map(memory_map)
        .font_data(ctx.context.font_data)
        .graphics_info(ctx.context.graphics_info)
        .framebuffer_data(ctx.context.frame_buffer)
        .runtime_service(ctx.context.runtime_service)
        .rsdp(ctx.context.rsdp)
        .dwarf_data(ctx.context.dwarf_data)
        .packed(ctx.context.packed)
        .kernel_elf(ctx.context.elf)
        .kernel_loaded_elf(ctx.context.loaded_kernel);

    let mut bridge = BootBridge::new(builder.build());

    mapper.transfer(&uefi_mapping, &mut bridge, &mut allocator, true);

    bridge.ptr()
}

fn extract_memory_map<'a>(entry_size: usize, memory_map: uefi::table::boot::MemoryMap<'a>) -> &'a [u8] {
    let entries = memory_map.entries();
    let start = memory_map.get(0).unwrap() as *const MemoryDescriptor as *const u8;
    let len = entries.len() * entry_size;
    unsafe { core::slice::from_raw_parts(start, len) }
}

pub fn prepare_kernel_page(mut ctx: InitializationContext<Stage2>) -> InitializationContext<Stage3> {
    let system_table = system_table();
    let boot_service = system_table.boot_services();
    let mut allocator = BootServiceFrameAlloc(boot_service);

    let p4_frame = allocator.allocate_frame::<Size4K>().unwrap();
    let p4_table = p4_frame.start_address().as_u64() as *mut Table<RootDirect>;
    let mut kernel_mapper = unsafe { Mapper::new_custom(p4_table) };

    let protocol = boot_service
        .open_protocol_exclusive::<LoadedImage>(system_table.boot_services().image_handle())
        .expect("Failed to open protocol for loaded image");

    let loaded_image_protocol = protocol.get().expect("Failed to get loaded image protocol");

    let (start, size) = loaded_image_protocol.info();
    let (start, size) = (PhysAddr::new(start as u64), size as usize);

    unsafe { kernel_mapper.identity_map_addr_auto(start, size, EntryFlags::PRESENT, &mut allocator) };
    let loaded_kernel = ctx.context_mut().elf.load_assume_writeable(&mut kernel_mapper, false, &mut allocator);

    kernel_mapper.p4_mut()[511].set(p4_frame, EntryFlags::PRESENT | EntryFlags::WRITABLE);

    // for ap bootstrap page table at 0x100000
    boot_service
        .allocate_pages(AllocateType::Address(0x100000), MemoryType::LOADER_DATA, 64)
        .expect("Failed to allocate pages for bootstrap kernel ap page tables");

    let tmp_alloc = boot_service
        .allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, 32)
        .expect("Failed to allocate pages");

    // We allocated from uefi so it's safe
    let tmp_alloc = unsafe { LinearAllocator::new(PhysAddr::new(tmp_alloc), 32 * Size4K::SIZE as usize) };

    ctx.next((p4_table, tmp_alloc, loaded_kernel))
}
