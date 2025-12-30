#![no_std]
#![no_main]

use core::panic::PanicInfo;

fn add(n: u64) -> u64 {
    if n > 20 {
        return 0;
    }

    add(n + 1)
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    add(10);

    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
