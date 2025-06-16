#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(radium::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use alloc::{sync::Arc, vec::Vec};
use bootbridge::RawBootBridge;
use radium::{smp::cpu_local, utils::mutex::Mutex};

const NUM_THREADS: usize = 64;
const NUM_INCREMENTS: usize = 100_000;

#[unsafe(no_mangle)]
pub extern "C" fn start(boot_bridge: *mut RawBootBridge) -> ! {
    radium::init(boot_bridge, test_main);
}

#[test_case]
fn mutex_increment() {
    let counter = Arc::new(Mutex::new(0usize));
    let mut handles = Vec::new();

    for _ in 0..NUM_THREADS {
        let counter_clone = Arc::clone(&counter);
        handles.push(cpu_local().local_scheduler().spawn(move || {
            for _ in 0..NUM_INCREMENTS {
                let mut lock = counter_clone.lock();
                *lock += 1;
            }
        }));
    }

    let expected = NUM_THREADS * NUM_INCREMENTS;

    for handle in handles {
        handle.join();
    }

    let result = *counter.lock();

    assert_eq!(
        result, expected,
        "Mutex did not protect the counter properly!"
    );
}
