#![no_std]
#![no_main]

use core::{arch::asm, panic::PanicInfo};

pub fn spawn(f: fn() -> !) {
    unsafe {
        asm!(
            "syscall",
            in("rax") 2,
            in("rdx") f as *const () as u64,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
}

fn syscall_sleep(amount_ms: usize) {
    unsafe {
        asm!(
            "syscall",
            in("rax") 1,
            in("rdx") amount_ms,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
}

fn syscall_exit_thread() -> ! {
    unsafe {
        asm!(
            "syscall",
            in("rax") 3,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }

    unreachable!("Sys exit thread doesn't work");
}

fn syscall_exit() -> ! {
    unsafe {
        asm!(
            "syscall",
            in("rax") 0,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }

    unreachable!("Sys exit doesn't work");
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    syscall_sleep(5000);
    for _ in 0..128 {
        spawn(|| {
            for _ in 0..1_000_000_00u64 {}
            syscall_exit_thread();
        });
    }
    syscall_exit();
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
