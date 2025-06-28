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
use pager::EntryFlags;
use radium::driver::uefi_runtime::uefi_runtime;
use radium::logger::LOGGER;
use radium::scheduler::{self, sleep, vsys_reg, VsysThread};
use radium::smp::cpu_local;
use radium::utils::mutex::Mutex;
use radium::{hlt_loop, print, println, serial_print, serial_println, DriverReslover};
use rstd::drivcall::{DRIVCALL_ERR_VSYSCALL_FULL, DRIVCALL_VSYS_REQ};
use santa::Elf;
use sentinel::log;

static TEST_MUTEX: Mutex<Vec<usize>> = Mutex::new(Vec::new());

#[unsafe(no_mangle)]
pub extern "C" fn start(boot_bridge: *mut RawBootBridge) -> ! {
    radium::init(boot_bridge, kmain_thread);
}

fn kmain_thread() {
    let packed = cpu_local()
        .ctx()
        .lock()
        .context_mut()
        .boot_bridge
        .packed_drivers();
    for driver in packed.iter() {
        let driver_elf = Elf::new(driver.data).expect("Driver elf not valid");
        let start = cpu_local().ctx().lock().map(
            driver_elf.max_memory_needed(),
            EntryFlags::WRITABLE | EntryFlags::NEEDS_REMAP,
        );

        let phys_start = cpu_local()
            .ctx()
            .lock()
            .context_mut()
            .active_table()
            .translate_page(start)
            .unwrap();
        log!(
            Trace,
            "driver loaded physical addr {:x}, virtual addr {:x}",
            phys_start.start_address(),
            start.start_address(),
        );
        unsafe { driver_elf.load(start.start_address().as_mut_ptr()) };
        driver_elf
            .apply_relocations(start.start_address(), &DriverReslover)
            .expect("");

        cpu_local().ctx().lock().virtually_map(
            &driver_elf,
            start.start_address(),
            phys_start.start_address(),
        );

        let init_fn = driver_elf
            .lookup_symbol("init", start.start_address())
            .expect("init fn not found in driver");
        let init_fn: extern "C" fn() =
            unsafe { core::mem::transmute(init_fn.as_u64()) };
        init_fn();
    }

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
    log!(Debug, "This should be the last log");
    LOGGER.flush_all(&[|s| serial_print!("{s}"), |s| print!("{s}")]);

    #[cfg(test)]
    test_main();

    hlt_loop();
}
