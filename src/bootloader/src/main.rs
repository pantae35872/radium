#![no_main]
#![no_std]
#![feature(str_from_raw_parts)]
#![feature(allocator_api)]

use core::arch::asm;

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
    let rsdp = system_table.config_table().iter().find(|e| {
        matches!(
            &e.guid.to_ascii_hex_lower(),
            b"eb9d2d30-2d88-11d3-9a16-0090273fc14d" | b"8868e871-e4f1-11d3-bc22-0080c73c8881",
        )
    });
    if let Some(rsdp) = rsdp {
        println!("Found rsdp: {:?}", rsdp);
    }

    let (entrypoint, boot_info, is_any_key_boot) = load_kernel(&mut system_table);
    if is_any_key_boot {
        any_key_boot(&mut system_table);
    }

    initialize_graphics_kernel(&mut system_table, boot_info);

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
