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
use radium::driver::pit::PIT;
use radium::driver::uefi_runtime::uefi_runtime;
use radium::interrupt::io_apic::RedirectionTableEntry;
use radium::interrupt::InterruptIndex;
use radium::logger::LOGGER;
use radium::scheduler::{
    interrupt_wait, pin, pinned, sleep, unpin, vsys_reg, VsysThread, DRIVCALL_ERR_VSYSCALL_FULL,
    DRIVCALL_VSYS_REQ,
};
use radium::smp::cpu_local;
use radium::utils::mutex::Mutex;
use radium::{hlt_loop, print, println, serial_print, serial_println};
use sentinel::log;

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
            log!(Info, "Waiting for threads...");
            let mut thread1 = VsysThread::new(1);
            log!(
                Info,
                "Handling 1: {}, with value sent: {}",
                thread1.global_id(),
                thread1.state.rcx
            );

            //pinned(|| {
            //    cpu_local().ctx().lock().redirect_legacy_irqs(
            //        0,
            //        RedirectionTableEntry::new(InterruptIndex::PITVector, cpu_local().core_id()),
            //    );
            //    PIT.get().unwrap().lock().dumb_wait_10ms_test();
            //    interrupt_wait(InterruptIndex::PITVector);
            //});

            thread1.state.rsi = thread1.state.rcx + 1;
        }
    });

    sleep(1000);
    for _ in 0..16 {
        cpu_local().local_scheduler().spawn(|| for send in 0..10 {
            log!(Info,
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
            log!(Info, "Received: {ret}");
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
    LOGGER.flush_all(&[|s| serial_print!("{s}")]);

    #[cfg(test)]
    test_main();

    hlt_loop();
}
