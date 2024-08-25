#![no_main]
#![no_std]
#![feature(str_from_raw_parts)]
#![feature(allocator_api)]

use core::arch::asm;

use alloc::format;
use boot_services::{read_config, read_file};
use common::{boot::BootInformation, toml::parser::TomlValue};
use elf_loader::load_elf;
use graphics::{initialize_graphics_bootloader, initialize_graphics_kernel};
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
pub mod elf_loader;
pub mod graphics;

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

    let kernel_file: &str = config
        .get("kernel_file")
        .expect("No kernel file found in info file")
        .as_string()
        .expect("Kernel file is not a string value in file info");
    let kernel_font_file: &str = config
        .get("font_file")
        .expect("No font file found in info file")
        .as_string()
        .expect("Font file is not a string value in file info");

    let kernel_font_buffer = read_file(&mut system_table, &format!("\\boot\\{}", kernel_font_file));
    let kernel_buffer = read_file(&mut system_table, &format!("\\boot\\{}", kernel_file));

    let boot_info = unsafe {
        &mut *(system_table
            .boot_services()
            .allocate_pool(MemoryType::LOADER_CODE, size_of::<BootInformation>())
            .unwrap_or_else(|e| panic!("Failed to allocate memory for the boot information {}", e))
            as *mut BootInformation)
    };
    boot_info.boot_info_start = boot_info as *mut BootInformation as u64;
    boot_info.boot_info_end =
        boot_info as *mut BootInformation as u64 + size_of::<BootInformation>() as u64 - 1;
    boot_info.font_start = kernel_font_buffer.as_ptr() as u64;
    boot_info.font_end = kernel_font_buffer.as_ptr() as u64 + kernel_font_buffer.len() as u64 - 1;

    let entrypoint = load_elf(&mut system_table, kernel_buffer, boot_info);
    if config
        .get("any_key_boot")
        .expect("any_key_boot boot config not found")
        .as_bool()
        .expect("any_key_boot is not a boolean")
    {
        any_key_boot(&mut system_table);
    }

    initialize_graphics_kernel(&mut system_table, boot_info);

    let (system_table, memory_map) = system_table.exit_boot_services(MemoryType::LOADER_CODE);
    boot_info.memory_map = memory_map;
    boot_info.runtime_system_table = system_table.get_current_system_table_addr();
    boot_info.largest_page = [
        boot_info.memory_map.entries().last().unwrap() as *const MemoryDescriptor as u64
            + size_of::<MemoryDescriptor>() as u64
            - 1,
        boot_info as *mut BootInformation as u64 + size_of::<BootInformation>() as u64 - 1,
        kernel_buffer.as_ptr() as u64 + kernel_buffer.len() as u64 - 1,
    ]
    .iter()
    .max()
    .unwrap()
        / 0x40000000
        + 1;
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
