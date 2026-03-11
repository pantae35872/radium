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
use pager::prepare_flags;
use sentinel::{LogLevel, LoggerBackend, disable_logger, set_logger};
use uefi::{
    Handle, Status, entry,
    table::{Boot, SystemTable, boot::MemoryType},
};

use uefi_services::{print, println};

extern crate alloc;

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
    disable_logger();

    let (system_table, memory_map) = system_table.exit_boot_services(MemoryType::LOADER_DATA);
    let stage5 = stage4.next((entry_size, system_table.as_ptr() as u64));
    let entrypoint = stage5.context().loaded_kernel.entry().as_u64();
    let table = stage5.context().table as u64;

    let boot_bridge = finialize_mapping(stage5, bootbridge_builder, memory_map);
    unsafe { prepare_flags() };

    unsafe {
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
