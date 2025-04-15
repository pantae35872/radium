#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![reexport_test_harness_main = "test_main"]
#![test_runner(radium::test_runner)]

extern crate radium;

use bootbridge::RawBootBridge;
use radium::println;

#[no_mangle]
pub extern "C" fn start(boot_bridge: *mut RawBootBridge) -> ! {
    radium::init(boot_bridge);
    test_main();
    loop {}
}

#[test_case]
fn test_println() {
    println!("test_println output");
}
