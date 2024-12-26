#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(radium::test_runner)]
#![reexport_test_harness_main = "test_main"]

use common::boot::BootInformation;
use radium::driver::uefi_runtime::uefi_runtime;

#[no_mangle]
pub extern "C" fn start(boot_info_address: *const BootInformation) -> ! {
    radium::init(boot_info_address);
    test_main();
    loop {}
}

#[test_case]
fn get_time() {
    assert!(uefi_runtime().get_time().is_some())
}
