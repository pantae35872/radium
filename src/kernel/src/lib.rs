#![no_std]
#![no_main]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]
#![feature(abi_x86_interrupt)]
#![feature(ptr_internals)]
#![feature(custom_test_frameworks)]
#![feature(core_intrinsics)]
#![feature(str_from_utf16_endian)]
#![feature(naked_functions)]
#![feature(pointer_is_aligned_to)]
#![feature(sync_unsafe_cell)]
#![feature(iter_next_chunk)]
#![feature(iter_array_chunks)]
#![feature(decl_macro)]
#![allow(internal_features)]
#![allow(undefined_naked_function_abi)]

#[macro_use]
extern crate bitflags;
extern crate alloc;
extern crate core;
extern crate lazy_static;
extern crate spin;

pub mod driver;
pub mod filesystem;
pub mod gdt;
pub mod graphics;
pub mod interrupt;
pub mod logger;
pub mod memory;
pub mod print;
pub mod serial;
pub mod task;
pub mod userland;
pub mod utils;

use core::panic::PanicInfo;

use bootbridge::{BootBridge, RawBootBridge};
use graphics::color::Color;
use graphics::BACKGROUND_COLOR;
use logger::LOGGER;

pub fn init(boot_bridge: *const RawBootBridge) {
    let boot_bridge = BootBridge::new(boot_bridge);
    memory::init(&boot_bridge);
    graphics::init(&boot_bridge);
    print::init(&boot_bridge, Color::new(209, 213, 219), BACKGROUND_COLOR);
    gdt::init_gdt();
    interrupt::init();
    x86_64::instructions::interrupts::enable();
    driver::init(&boot_bridge);
    userland::init();
}

#[cfg(test)]
#[no_mangle]
pub extern "C" fn start(boot_info: *mut RawBootBridge) -> ! {
    init(boot_info);
    test_main();
    hlt_loop();
}

pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

pub trait Testable {
    fn run(&self);
}

pub const TESTING: bool = cfg!(test) | cfg!(feature = "testing");

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    log!(Critical, "{}", info);

    LOGGER.flush_all(if print::DRIVER.get().is_none() {
        log!(
            Warning,
            "Screen print not avaiable logging into serial ports"
        );
        &[|s| serial_print!("{s}")]
    } else if TESTING {
        &[|s| serial_print!("{s}")]
    } else {
        &[|s| serial_print!("{s}"), |s| print!("{s}")]
    });

    if TESTING {
        test_panic_handler(info);
    } else {
        hlt_loop();
    }
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

pub fn test_runner(_tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", _tests.len());
    for test in _tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

pub fn test_panic_handler(info: &PanicInfo) -> ! {
    serial_println!("[failed]");
    exit_qemu(QemuExitCode::Failed);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }

    // Wait for qemu to exit
    hlt_loop();
}
