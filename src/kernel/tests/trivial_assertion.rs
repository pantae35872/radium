#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(radium::test_runner)]
#![reexport_test_harness_main = "test_main"]

use bootbridge::RawBootBridge;

#[no_mangle]
pub extern "C" fn start(boot_bridge: *const RawBootBridge) -> ! {
    radium::init(boot_bridge);
    test_main();
    loop {}
}

#[test_case]
fn trivial_assertion() {
    assert_eq!(1, 1);
}
