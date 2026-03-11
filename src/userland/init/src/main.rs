#![no_std]
#![no_main]

use core::{
    arch::asm,
    panic::PanicInfo,
    sync::atomic::{AtomicUsize, Ordering},
};

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

static COUNT: AtomicUsize = AtomicUsize::new(0);

#[inline(always)]
pub fn is_stack_aligned_16() -> bool {
    let rsp: usize;

    unsafe {
        core::arch::asm!(
            "mov {}, rsp",
            out(reg) rsp,
            options(nomem, nostack, preserves_flags)
        );
    }

    rsp & 0xF == 0
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    if !is_stack_aligned_16() {
        syscall_exit();
    }

    syscall_sleep(10000);
    for _ in 0..512 {
        spawn(|| {
            if !is_stack_aligned_16() {
                syscall_exit();
            }

            for _ in 0..1_000_000 {
                COUNT.fetch_add(1, Ordering::Relaxed);
            }
            syscall_exit_thread();
        });
    }

    while COUNT.load(Ordering::Relaxed) < 1_000_000 * 512 {
        core::hint::spin_loop();
    }
    syscall_exit();
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
