#![no_main]
#![no_std]
#![feature(str_from_raw_parts)]
#![feature(allocator_api)]

use core::arch::asm;

use alloc::vec;
use alloc::vec::Vec;
use boot_cfg_parser::toml::parser::TomlValue;
use boot_services::read_config;
use bootbridge::BootBridgeBuilder;
use graphics::{initialize_graphics_bootloader, initialize_graphics_kernel};
use kernel_loader::load_kernel;
use uefi::{
    entry,
    table::{
        boot::{self, MemoryDescriptor, MemoryMap, MemoryType},
        Boot, SystemTable,
    },
    Handle, Status,
};

use uefi_services::{println, system_table};
extern crate alloc;

pub mod boot_services;
pub mod elf_loader;
pub mod graphics;
pub mod kernel_loader;

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
            .allocate_pool(MemoryType::LOADER_CODE, size)
            .unwrap_or_else(|e| panic!("Failed to allocate memory for the boot information {}", e))
    });

    initialize_graphics_bootloader(&mut system_table);

    let config: TomlValue = read_config(&mut system_table, "\\boot\\bootinfo.toml");

    let entrypoint = load_kernel(&mut system_table, &mut boot_bridge, &config);

    if config
        .get("any_key_boot")
        .expect("any_key_boot boot config not found")
        .as_bool()
        .expect("any_key_boot is not a boolean")
    {
        any_key_boot(&mut system_table);
    }

    initialize_graphics_kernel(&mut system_table, &mut boot_bridge, &config);
    let entry_size = system_table.boot_services().memory_map_size().entry_size;

    let (system_table, memory_map) = system_table.exit_boot_services(MemoryType::LOADER_CODE);
    let entries = memory_map.entries();
    let start = memory_map.get(0).unwrap() as *const MemoryDescriptor as *const u8;
    let len = entries.len() * core::mem::size_of::<boot::MemoryDescriptor>();
    let memory_map_bytes: &[u8] = unsafe { core::slice::from_raw_parts(start, len) };
    boot_bridge.memory_map(memory_map_bytes, entry_size);
    let boot_bridge = boot_bridge.build().expect("Failed to build boot bridge");

    unsafe {
        asm!(
            r#"
            jmp {}
        "#,
        in(reg) entrypoint,
        in("rdi") boot_bridge
        );
    }

    unreachable!();
}
