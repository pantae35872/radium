#![no_std]

use core::panic::PanicInfo;

use sentinel::{log, set_logger, LoggerBackend};

#[unsafe(no_mangle)]
pub extern "C" fn init() {
    set_logger(unsafe { get_klogger() });

    log!(Info, "Test driver initialized");
}

unsafe extern "Rust" {
    fn get_klogger() -> &'static dyn LoggerBackend;
    fn kpanic(info: &PanicInfo) -> !;
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe { kpanic(info) }
}
