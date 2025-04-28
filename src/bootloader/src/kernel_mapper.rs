use core::ptr::write_bytes;

use alloc::vec;
use bootbridge::BootBridgeBuilder;
use pager::{
    address::{Frame, PhysAddr},
    allocator::{linear_allocator::LinearAllocator, FrameAllocator},
    page_table_size,
    paging::{
        table::{DirectLevel4, Table},
        ActivePageTable, Entry,
    },
    EntryFlags, Mapper, PageLevel, PAGE_SIZE,
};
use santa::Elf;
use uefi::{
    proto::loaded_image::LoadedImage,
    table::boot::{AllocateType, MemoryType},
};
use uefi_services::{println, system_table};

use crate::config::BootConfig;

pub fn prepare_kernel_page(
    config: &BootConfig,
    boot_bridge: &mut BootBridgeBuilder<impl Fn(usize) -> *mut u8>,
    elf: &Elf<'static>,
    kernel_phys_start: PhysAddr,
) -> (u64, LinearAllocator) {
    let system_table = system_table();

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
    println!("{mem_map_size:#x} {mem_map_pages:#x}");

    let kernel_pages_table = system_table
        .boot_services()
        .allocate_pages(
            AllocateType::AnyPages,
            MemoryType::LOADER_DATA,
            mem_map_pages,
        )
        .expect("Failed to allocate pages for kernel early page tables");

    println!(
        "KERNEL PAGE TABLE PHYSADDR: [{:#x}-{:#x}]",
        kernel_pages_table as u64,
        kernel_pages_table as usize + mem_map_size - 1
    );

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

    kernel_table.virtually_map_object(elf, kernel_phys_start, &mut kernel_page_allocator);

    let protocol = system_table
        .boot_services()
        .open_protocol_exclusive::<LoadedImage>(system_table.boot_services().image_handle())
        .expect("Failed to open protocol for loaded image");

    let loaded_image_protocol = protocol.get().expect("Failed to get loaded image protocol");

    let (start, size) = loaded_image_protocol.info();
    println!(
        "BOOTLOADER RANGE: [{:#x}-{:#x}]",
        start as u64,
        start as u64 + size
    );
    unsafe {
        kernel_table
            .mapper_with_allocator(&mut kernel_page_allocator)
            .identity_map_by_size(
                Frame::containing_address(PhysAddr::new(start as u64)),
                size as usize,
                EntryFlags::PRESENT,
            );
    };

    boot_bridge.kernel_base(kernel_phys_start);

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

    (kernel_pages_table, kernel_page_allocator)
}
