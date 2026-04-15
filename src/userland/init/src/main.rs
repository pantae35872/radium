#![no_std]
#![no_main]

use core::{
    arch::asm,
    fmt,
    hint::black_box,
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

fn syscall_test(c: char) {
    unsafe {
        asm!(
            "syscall",
            in("rax") 4,
            in("rdx") c as u64,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
}

fn syscall_flush_log() {
    unsafe {
        asm!(
            "syscall",
            in("rax") 5,
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

fn computation() -> ! {
    let mut x: u64 = 0x1234_5678_9ABC_DEF0;
    let mut y: u64 = 0xCAFEBABEDEADBEEF;
    let mut i: u64 = 0;
    loop {
        x ^= x.rotate_left((i % 63) as u32);
        x = x.wrapping_mul(0x9E3779B97F4A7C15);
        x = x.wrapping_add(i);

        // more meaningless mixing
        y ^= y.rotate_right((x % 59) as u32);
        y = y.wrapping_mul(0xD6E8FEB86659FD93);
        y = y.wrapping_add(x ^ i);

        // pointless branch
        if (x ^ y) & 1 == 0 {
            x = x.wrapping_add(y.rotate_left(7));
        } else {
            y = y.wrapping_add(x.rotate_right(11));
        }

        i += i.wrapping_add(1);
        syscall_test(' ');

        black_box((x, y));
    }
}

struct SyscallWriter;

impl fmt::Write for SyscallWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            syscall_test(c);
        }
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: ::core::fmt::Arguments) {
    use core::fmt::Write;
    let _ = SyscallWriter.write_fmt(args);
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::_print(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! println {
    () => ($crate::serial_print!("\n"));
    ($($arg:tt)*) => {
        $crate::_print(format_args!("{}\n", format_args!($($arg)*)))
    };
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    println!("counting..");
    syscall_sleep(3000);

    for _ in 0..512 {
        spawn(|| {
            for _ in 0..1_000_000 {
                COUNT.fetch_add(1, Ordering::Relaxed);
            }

            syscall_exit_thread();
        });
    }

    let mut timeout = 0;
    while COUNT.load(Ordering::Relaxed) < 1_000_000 * 512 && timeout < 10 {
        //core::hint::spin_loop();
        syscall_sleep(1000);
        println!("{}", COUNT.load(Ordering::Relaxed));
        timeout += 1;
    }

    if timeout >= 10 {
        println!("Failed");
        syscall_flush_log();
    } else {
        syscall_flush_log();
        println!("Finished {}", COUNT.load(Ordering::SeqCst));
    }

    syscall_exit();
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
