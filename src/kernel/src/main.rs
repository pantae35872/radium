#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(radium::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;
extern crate core;
extern crate lazy_static;
extern crate radium;
extern crate spin;

use core::arch::asm;

use alloc::vec::Vec;
use bootbridge::RawBootBridge;
use radium::driver::uefi_runtime::uefi_runtime;
use radium::logger::LOGGER;
use radium::scheduler::{
    sleep, vsys_reg, VsysThread, DRIVCALL_ERR_VSYSCALL_FULL, DRIVCALL_VSYS_REQ,
};
use radium::smp::cpu_local;
use radium::utils::mutex::Mutex;
use radium::{hlt_loop, print, println, serial_print, serial_println};
use sentinel::log;

// TODO: Implements acpi to get io apic
// TODO: Use ahci interrupt (needs io apic) with waker
// TODO: Implements waker based async mutex
// TODO: Impelemnts kernel services executor

static TEST_MUTEX: Mutex<Vec<usize>> = Mutex::new(Vec::new());

#[unsafe(no_mangle)]
pub extern "C" fn start(boot_bridge: *mut RawBootBridge) -> ! {
    radium::init(boot_bridge, kmain_thread);
}

fn kmain_thread() {
    println!("Hello, world!!!, from kmain thread");
    log!(Info, "Time {:?}", uefi_runtime().lock().get_time());

    cpu_local().local_scheduler().spawn(|| {
        vsys_reg(1); // VSYS 1
        loop {
            println!("Waiting for threads...");
            let mut thread1 = VsysThread::new(1);
            println!(
                "Handling 1: {}, with value sent: {}",
                thread1.global_id(),
                thread1.state.rcx
            );

            thread1.state.rsi = thread1.state.rcx + 1;
        }
    });

    for _ in 0..16 {
        cpu_local().local_scheduler().spawn(|| for send in 0..10 {
            serial_println!(
                "Sending request from id {}...",
                cpu_local().current_thread_id()
            );

            let ret: u64;
            let res: u64;
            unsafe {
                asm!("int 0x90", in("rdi") DRIVCALL_VSYS_REQ, in("rax") 1, in("rcx") send, out("rsi") ret, lateout("rdi") res);
            }

            if res == DRIVCALL_ERR_VSYSCALL_FULL {
                serial_println!("SYSCALL FULL");
                continue;
            }

            assert_eq!(ret, send + 1);
            serial_println!("Received: {ret}");
        });
    }

    cpu_local().local_scheduler().spawn(|| {
        for i in 0..64 {
            serial_println!(
                "Thread {} [{i}]: popped {:?}",
                cpu_local().current_thread_id(),
                TEST_MUTEX.lock().pop()
            );
            sleep(100);
        }
    });
    cpu_local().local_scheduler().spawn(|| {
        for i in 0..64 {
            serial_println!(
                "Thread {} [{i}]: trying to push",
                cpu_local().current_thread_id()
            );
            sleep(100);
            TEST_MUTEX.lock().push(i * 10);
            serial_println!("Thread {} [{i}]: pushed", cpu_local().current_thread_id());
        }
    });

    sleep(5000);

    cpu_local().local_scheduler().spawn(|| {
        log!(
            Debug,
            "this should be thread 2, current tid {}",
            cpu_local().current_thread_id()
        );
    });

    log!(Debug, "This should be the last log");
    LOGGER.flush_all(&[|s| serial_print!("{s}"), |s| print!("{s}")]);

    #[cfg(test)]
    test_main();

    hlt_loop();
}
