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

use bootbridge::RawBootBridge;
use radium::driver::uefi_runtime::{uefi_runtime, EfiStatus, ResetType};
use radium::logger::LOGGER;
use radium::scheduler::sleep;
use radium::smp::cpu_local;
use radium::{hlt_loop, print, println, serial_print, serial_println};
use sentinel::log;

// TODO: Implements acpi to get io apic
// TODO: Use ahci interrupt (needs io apic) with waker
// TODO: Implements waker based async mutex
// TODO: Impelemnts kernel services executor

#[unsafe(no_mangle)]
pub extern "C" fn start(boot_bridge: *mut RawBootBridge) -> ! {
    radium::init(boot_bridge);
    cpu_local().local_scheduler().spawn(|| kmain_thread());

    LOGGER.flush_all(&[|s| serial_print!("{s}"), |s| print!("{s}")]);

    unsafe { cpu_local().set_tid(usize::MAX) }; // Set tid to usize::MAX to start scheduling

    hlt_loop();
}

fn kmain_thread() {
    println!("Hello, world!!!, from kmain thread");
    log!(Info, "Time {:?}", uefi_runtime().lock().get_time());
    cpu_local().local_scheduler().spawn(|| {
        for i in 0..64 {
            println!(
                "hello from thread: {}, {i}",
                cpu_local().current_thread_id()
            );
            println!("{:?}", uefi_runtime().lock().get_time());
            sleep(1000);
        }
    });
    cpu_local().local_scheduler().spawn(|| {
        for i in 0..64 {
            serial_println!(
                "hello from thread: {}, {i}",
                cpu_local().current_thread_id()
            );
            sleep(1000);
        }
    });

    #[cfg(test)]
    test_main();

    hlt_loop();
}
