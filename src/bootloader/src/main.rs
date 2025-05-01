#![no_main]
#![no_std]
#![feature(str_from_raw_parts)]
#![feature(allocator_api)]

use core::arch::asm;

use boot_cfg_parser::toml::parser::TomlValue;
use boot_services::LoaderFile;
use bootbridge::{BootBridge, BootBridgeBuilder};
use config::BootConfig;
use graphics::{initialize_graphics_bootloader, initialize_graphics_kernel};
use kernel_loader::load_kernel;
use pager::{
    address::{PhysAddr, VirtAddr},
    paging::{
        table::{DirectLevel4, RecurseLevel4, Table},
        ActivePageTable,
    },
    registers::{Cr0, Cr0Flags, Cr3Flags, Efer, EferFlags, CS},
    EntryFlags, Mapper, KERNEL_DIRECT_PHYSICAL_MAP, PAGE_SIZE,
};
use uefi::{
    entry,
    table::{
        boot::{MemoryDescriptor, MemoryType},
        Boot, SystemTable,
    },
    Handle, Status,
};

use uefi_services::println;
extern crate alloc;

pub mod boot_services;
pub mod config;
pub mod elf_loader;
pub mod graphics;
pub mod kernel_loader;
pub mod kernel_mapper;

fn any_key_boot(system_table: &mut SystemTable<Boot>) {
    println!("press any key to boot...");

    loop {
        match system_table.stdin().read_key() {
            Ok(key) => match key {
                Some(_) => break,
                None => {}
            },
            Err(err) => {
                panic!("failed to read key: {}", err);
            }
        }
    }
}

#[entry]
fn main(handle: Handle, mut system_table: SystemTable<Boot>) -> Status {
    uefi_services::init(&mut system_table).unwrap();

    let mut boot_bridge = BootBridgeBuilder::new(|size: usize| {
        uefi_services::system_table()
            .boot_services()
            .allocate_pool(MemoryType::RUNTIME_SERVICES_DATA, size)
            .unwrap_or_else(|e| panic!("Failed to allocate memory for the boot information {}", e))
    });

    initialize_graphics_bootloader(&mut system_table);

    let config: TomlValue = LoaderFile::new("\\boot\\bootinfo.toml").into();
    let config: BootConfig = BootConfig::parse(&config);

    let (entrypoint, table, mut allocator) = load_kernel(&mut boot_bridge, &config);

    if config.any_key_boot() {
        any_key_boot(&mut system_table);
    }

    initialize_graphics_kernel(&mut system_table, &mut boot_bridge, &config);
    let entry_size = system_table.boot_services().memory_map_size().entry_size;

    let (system_table, mut memory_map) =
        system_table.exit_boot_services(MemoryType::RUNTIME_SERVICES_DATA);

    memory_map.sort();
    boot_bridge.runtime_service(system_table.as_ptr() as u64);

    let entries = memory_map.entries();
    let start = memory_map.get(0).unwrap() as *const MemoryDescriptor as *const u8;
    let len = entries.len() * entry_size;
    let memory_map_bytes: &[u8] = unsafe { core::slice::from_raw_parts(start, len) };

    let mut kernel_table =
        unsafe { ActivePageTable::new_custom(table as *mut Table<DirectLevel4>) };
    kernel_table.identity_map_object(&boot_bridge, &mut allocator);
    kernel_table.identity_map_object(
        &bootbridge::MemoryMap::new(
            memory_map_bytes,
            entry_size,
            MemoryDescriptor::VERSION as usize,
        ),
        &mut allocator,
    );

    assert!(memory_map
        .entries()
        .next()
        .is_some_and(|e| e.phys_start == 0));

    for usable in memory_map
        .entries()
        .filter(|e| e.ty == MemoryType::CONVENTIONAL)
    {
        let size = (usable.page_count * PAGE_SIZE) as usize;
        unsafe {
            kernel_table
                .mapper_with_allocator(&mut allocator)
                .map_to_range_by_size(
                    VirtAddr::new(KERNEL_DIRECT_PHYSICAL_MAP.as_u64() + usable.phys_start).into(),
                    PhysAddr::new(usable.phys_start).into(),
                    size,
                    EntryFlags::WRITABLE,
                )
        };
    }

    boot_bridge.memory_map(
        memory_map_bytes,
        entry_size,
        MemoryDescriptor::VERSION as usize,
    );

    let boot_bridge = boot_bridge.build().expect("Failed to build boot bridge");

    unsafe {
        Efer::write_or(EferFlags::NoExecuteEnable);
        Cr0::write_or(Cr0Flags::WriteProtect);
        asm!(
        r#"
            mov cr3, {}
            jmp {}
        "#,
        in(reg) table,
        in(reg) entrypoint,
        in("rdi") boot_bridge
        );
    }

    unreachable!();
}
