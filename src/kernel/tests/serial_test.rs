#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(nothingos::test_runner)]
#![reexport_test_harness_main = "test_main"]

use common::boot::BootInformation;
use nothingos::serial_println;

#[no_mangle]
pub extern "C" fn start(multiboot_information_address: *mut BootInformation) -> ! {
    nothingos::init(multiboot_information_address);
    test_main();
    loop {}
}

#[test_case]
fn print_serial() {
    serial_println!("Test serial");
}
