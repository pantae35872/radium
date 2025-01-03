#![no_main]
#![no_std]
#![feature(str_from_raw_parts)]
#![feature(allocator_api)]

use core::arch::asm;

use boot_services::read_config;
use common::toml::parser::TomlValue;
use graphics::{initialize_graphics_bootloader, initialize_graphics_kernel};
use kernel_loader::load_kernel;
use uefi::{
    entry,
    table::{boot::MemoryType, Boot, SystemTable},
    Handle, Status,
};

use uefi_services::println;
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

    initialize_graphics_bootloader(&mut system_table);

    let config: TomlValue = read_config(&mut system_table, "\\boot\\bootinfo.toml");

    let (entrypoint, boot_info, is_any_key_boot) = load_kernel(&mut system_table, &config);
    if is_any_key_boot {
        any_key_boot(&mut system_table);
    }

    initialize_graphics_kernel(&mut system_table, boot_info, &config);

    let (system_table, memory_map) = system_table.exit_boot_services(MemoryType::LOADER_CODE);
    boot_info.init_memory(memory_map, system_table.get_current_system_table_addr());

    unsafe {
        asm!(
            r#"
            jmp {}
        "#,
        in(reg) entrypoint,
        in("rdi") boot_info
        );
    }

    unreachable!();
}
