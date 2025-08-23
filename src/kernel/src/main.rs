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
use radium::scheduler::{self, VsysThread, sleep, vsys_reg};
use radium::smp::cpu_local;
use radium::utils::mutex::Mutex;
use radium::{print, println, serial_print, serial_println};
use rstd::drivcall::{DRIVCALL_ERR_VSYSCALL_FULL, DRIVCALL_VSYS_REQ};
use sentinel::log;

static TEST_MUTEX: Mutex<Vec<usize>> = Mutex::new(Vec::new());

#[unsafe(no_mangle)]
pub extern "C" fn start(boot_bridge: *mut RawBootBridge) -> ! {
    radium::init(boot_bridge, kmain_thread);
}

fn kmain_thread() {
    #[cfg(test)]
    test_main();

    println!("Hello, world!!!, from kmain thread");

    scheduler::spawn(|| {
        vsys_reg(1); // VSYS 1
        loop {
            log!(Trace, "Waiting for threads...");
            let mut thread1 = VsysThread::new(1);
            log!(
                Trace,
                "Handling 1: {}, with value sent: {}",
                thread1.global_id(),
                thread1.state.rcx
            );

            thread1.state.rsi = thread1.state.rcx + 1;
        }
    });

    sleep(1000);

    for _ in 0..16 {
        scheduler::spawn(|| {
            for send in 0..10 {
                log!(
                    Trace,
                    "Sending request from id {}...",
                    cpu_local().current_thread_id()
                );

                let ret: u64;
                let res: u64;
                unsafe {
                    asm!("int 0x90", in("rdi") DRIVCALL_VSYS_REQ, in("rax") 1, in("rcx") send, out("rsi") ret, lateout("rdi") res);
                }

                assert!(res != DRIVCALL_ERR_VSYSCALL_FULL);
                assert_eq!(ret, send + 1);
                log!(Trace, "Received: {ret}");
            }
        });
    }

    let mut handles = Vec::new();
    handles.push(scheduler::spawn(|| {
        for i in 0..64 {
            serial_println!(
                "Thread {} [{i}]: popped {:?}",
                cpu_local().current_thread_id(),
                TEST_MUTEX.lock().pop()
            );
            sleep(100);
        }
    }));
    handles.push(scheduler::spawn(|| {
        for i in 0..64 {
            serial_println!(
                "Thread {} [{i}]: trying to push",
                cpu_local().current_thread_id()
            );
            sleep(100);
            TEST_MUTEX.lock().push(i * 10);
            serial_println!("Thread {} [{i}]: pushed", cpu_local().current_thread_id());
        }
    }));

    handles.push(scheduler::spawn(|| {
        log!(
            Debug,
            "this should be thread 2, current tid {}",
            cpu_local().current_thread_id()
        );
    }));

    for handle in handles {
        handle.join();
    }

    log!(Info, "Time {:?}", uefi_runtime().lock().get_time());
    {
        let ctx = cpu_local().ctx().lock();
        let allocator = ctx.context().buddy_allocator();
        log!(
            Info,
            "Usable memory left: {:.2} GB",
            (allocator.max_mem() - allocator.allocated()) as f32 / (1 << 30) as f32 // TO GB
        );
    }
    log!(Debug, "This should be the last log");
    LOGGER.flush_all(&[|s| serial_print!("{s}"), |s| print!("{s}")]);
}
