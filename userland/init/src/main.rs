#![no_std]
#![no_main]

use core::{arch::asm, panic::PanicInfo};

fn add(n: u64) -> u64 {
    if n > 20 {
        return n;
    }

    add(n + 1)
}

fn syscall_test(n: u64) {
    unsafe {
        asm!("syscall", in("r9") n, options(nostack));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    syscall_test(add(10));
    syscall_test(add(10));
    syscall_test(add(10));
    syscall_test(add(10));

    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
