#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(radium::test_runner)]
#![reexport_test_harness_main = "test_main"]

use common::boot::BootInformation;

#[no_mangle]
pub extern "C" fn start(multiboot_information_address: *mut BootInformation) -> ! {
    radium::init(multiboot_information_address);
    test_main();
    loop {}
}

#[test_case]
fn trivial_assertion() {
    assert_eq!(1, 1);
}
