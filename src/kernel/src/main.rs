#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(radium::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;
extern crate core;
extern crate lazy_static;
extern crate radium;
extern crate spin;

use bootbridge::RawBootBridge;
use radium::logger::LOGGER;
use radium::{hlt_loop, print, println, serial_print};

// TODO: Implements acpi to get io apic
// TODO: Use ahci interrupt (needs io apic) with waker
// TODO: Implements waker based async mutex
// TODO: Impelemnts kernel services executor

#[unsafe(no_mangle)]
pub extern "C" fn start(boot_bridge: *mut RawBootBridge) -> ! {
    hlt_loop();
    radium::init(boot_bridge);
    println!("Hello, world!!!");
    //#[cfg(not(feature = "testing"))]
    LOGGER.flush_all(&[|s| serial_print!("{s}"), |s| print!("{s}")]);
    //println!("Time Test: {:?}", uefi_runtime().get_time());
    #[cfg(test)]
    test_main();

    hlt_loop();
}
