#![no_main]
#![no_std]
#![feature(str_from_raw_parts)]
#![feature(allocator_api)]
#![allow(dead_code)]

use core::arch::asm;

use bootbridge::BootBridgeBuilder;
use graphics::{initialize_graphics_bootloader, initialize_graphics_kernel};
use kernel_loader::{load_kernel_elf, load_kernel_infos};
use kernel_mapper::{finialize_mapping, prepare_kernel_page};
use pager::registers::{Cr0, Efer};
use sentinel::{LogLevel, LoggerBackend, set_logger};
use uefi::{
    Handle, Status, entry,
    table::{Boot, SystemTable, boot::MemoryType},
};

use uefi_services::{print, println};

extern crate alloc;

mod config {
    include!(concat!(env!("OUT_DIR"), "/config.rs"));
}

pub mod boot_services;
pub mod context;
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

struct BasicUEFILogger;

impl LoggerBackend for BasicUEFILogger {
    fn log(&self, module_path: &'static str, level: sentinel::LogLevel, formatter: core::fmt::Arguments) {
        print!(
            "[{}] <- [{module_path}] : {formatter}",
            match level {
                LogLevel::Debug => "DEBUG",
                LogLevel::Info => "INFO",
                LogLevel::Trace => "TRACE",
                LogLevel::Error => "ERROR",
                LogLevel::Warning => "WARNING",
                LogLevel::Critical => "CRITICAL",
                _ => unreachable!(),
            }
        );
    }
}

static UEFI_LOGGER: BasicUEFILogger = BasicUEFILogger;

#[entry]
fn main(handle: Handle, mut system_table: SystemTable<Boot>) -> Status {
    uefi_services::init(&mut system_table).unwrap();

    initialize_graphics_bootloader(&mut system_table);
    set_logger(&UEFI_LOGGER);

    let bootbridge_builder = BootBridgeBuilder::new(|size: usize| {
        uefi_services::system_table()
            .boot_services()
            .allocate_pool(MemoryType::LOADER_DATA, size)
            .unwrap_or_else(|e| panic!("Failed to allocate memory for the boot information {}", e))
    });

    let stage1 = load_kernel_infos();
    let stage2 = load_kernel_elf(stage1);
    let stage3 = prepare_kernel_page(stage2);

    if config::config().boot_loader.any_key_boot {
        any_key_boot(&mut system_table);
    }

    let stage4 = initialize_graphics_kernel(stage3);

    let entry_size = system_table.boot_services().memory_map_size().entry_size;
    let (system_table, mut memory_map) = system_table.exit_boot_services(MemoryType::LOADER_DATA);
    memory_map.sort();
    let stage5 = stage4.next((entry_size, system_table.as_ptr() as u64));

    let stage6 = finialize_mapping(stage5, memory_map);

    let entrypoint = stage6.context().entry_point;
    let table = stage6.context().table;
    let boot_bridge = stage6.build_bridge(bootbridge_builder);

    unsafe {
        Efer::NoExecuteEnable.write_retained();
        Cr0::WriteProtect.write_retained();
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
