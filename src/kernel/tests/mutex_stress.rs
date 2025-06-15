#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(radium::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use alloc::sync::Arc;
use bootbridge::RawBootBridge;
use radium::{scheduler::sleep, serial_println, smp::cpu_local, utils::mutex::Mutex};

const NUM_THREADS: usize = 16;
const NUM_INCREMENTS: usize = 100_000;

#[unsafe(no_mangle)]
pub extern "C" fn start(boot_bridge: *mut RawBootBridge) -> ! {
    radium::init(boot_bridge, test_main);
}

#[test_case]
fn mutex_increment() {
    let counter = Arc::new(Mutex::new(0usize));

    for _ in 0..NUM_THREADS {
        let counter_clone = Arc::clone(&counter);
        cpu_local().local_scheduler().spawn(move || {
            for _ in 0..NUM_INCREMENTS {
                let mut lock = counter_clone.lock();
                serial_println!("{}", *lock);
                *lock += 1;
            }
        });
    }

    let expected = NUM_THREADS * NUM_INCREMENTS;

    sleep(320000);

    let result = *counter.lock();

    serial_println!("Final count: {}", result);
    serial_println!("Expected: {}", expected);
    assert_eq!(
        result, expected,
        "Mutex did not protect the counter properly!"
    );
}
