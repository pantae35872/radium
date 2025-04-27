use core::ptr::write_bytes;

use pager::{
    address::{Frame, PhysAddr},
    allocator::{linear_allocator::LinearAllocator, FrameAllocator},
    gdt::{Descriptor, Gdt},
    paging::{
        table::{DirectLevel4, Table},
        ActivePageTable, Entry,
    },
    registers::SegmentSelector,
    EntryFlags, Mapper, PAGE_SIZE,
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
    elf: &Elf<'static>,
    kernel_phys_start: PhysAddr,
) -> u64 {
    let system_table = system_table();
    let kernel_pages_table = system_table
        .boot_services()
        .allocate_pages(
            AllocateType::AnyPages,
            MemoryType::LOADER_DATA,
            config.early_boot_kernel_page_table_page_count(),
        )
        .expect("Failed to allocate pages for kernel early page tables");

    println!(
        "KERNEL PAGE TABLE PHYSADDR: [{:#x}-{:#x}]",
        kernel_pages_table as u64,
        kernel_pages_table as usize
            + config.early_boot_kernel_page_table_page_count() * PAGE_SIZE as usize
            - 1
    );

    unsafe {
        write_bytes(
            kernel_pages_table as *mut u8,
            0,
            config.early_boot_kernel_page_table_byte_count(),
        );
    }

    let mut kernel_page_allocator = unsafe {
        LinearAllocator::new(
            PhysAddr::new(kernel_pages_table),
            config.early_boot_kernel_page_table_byte_count(),
        )
    };

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
    // Do a recursive map
    kernel_table.p4_mut()[511] = Entry(
        p4_frame.start_address().as_u64() | (EntryFlags::PRESENT | EntryFlags::WRITABLE).bits(),
    );

    kernel_pages_table
}
