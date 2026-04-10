#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(radium::test_runner)]
#![reexport_test_harness_main = "test_main"]

#[unsafe(no_mangle)]
pub extern "C" fn start(boot_bridge: *mut bootbridge::RawBootBridge) -> ! {
    radium::init(boot_bridge);
}
