#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![reexport_test_harness_main = "test_main"]
#![test_runner(radium::test_runner)]

extern crate radium;

use common::boot::BootInformation;
use radium::println;

#[no_mangle]
pub extern "C" fn start(multiboot_information_address: *mut BootInformation) -> ! {
    radium::init(multiboot_information_address);
    test_main();
    loop {}
}

#[test_case]
fn test_println() {
    println!("test_println output");
}
