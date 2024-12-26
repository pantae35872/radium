#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![feature(pointer_is_aligned_to)]
#![reexport_test_harness_main = "test_main"]
#![test_runner(radium::test_runner)]

extern crate alloc;
extern crate radium;

use common::boot::BootInformation;
use radium::utils::circular_ring_buffer::CircularRingBuffer;

#[no_mangle]
pub extern "C" fn start(boot_info_address: *mut BootInformation) -> ! {
    radium::init(boot_info_address);
    test_main();
    loop {}
}

#[test_case]
pub fn read_write() {
    let buffer = CircularRingBuffer::<_, 5>::new();
    buffer.write(30);
    buffer.write(20);
    assert!(buffer.read().is_some_and(|e| e == 30));
    assert!(buffer.read().is_some_and(|e| e == 20));
    assert!(buffer.read().is_none());
    buffer.write(40);
    buffer.write(50);
    assert!(buffer.read().is_some_and(|e| e == 40));
    assert!(buffer.read().is_some_and(|e| e == 50));
    assert!(buffer.read().is_none());
}

#[test_case]
pub fn read_write_overwrite() {
    let buffer = CircularRingBuffer::<_, 5>::new();
    buffer.write(30);
    buffer.write(20);
    buffer.write(40);
    buffer.write(50);
    buffer.write(60);
    buffer.write(70);
    assert!(buffer.read().is_some_and(|e| e == 20));
    assert!(buffer.read().is_some_and(|e| e == 40));
    assert!(buffer.read().is_some_and(|e| e == 50));
    assert!(buffer.read().is_some_and(|e| e == 60));
    assert!(buffer.read().is_some_and(|e| e == 70));
    assert!(buffer.read().is_none());
}

#[test_case]
pub fn interleaved_read_write() {
    let buffer = CircularRingBuffer::<_, 5>::new();

    buffer.write(10);
    buffer.write(20);

    assert!(buffer.read().is_some_and(|e| e == 10));

    buffer.write(30);
    buffer.write(40);
    buffer.write(50);

    assert!(buffer.read().is_some_and(|e| e == 20));
    assert!(buffer.read().is_some_and(|e| e == 30));
    assert!(buffer.read().is_some_and(|e| e == 40));
    assert!(buffer.read().is_some_and(|e| e == 50));
    assert!(buffer.read().is_none());
}

#[test_case]
pub fn sequential_read_write() {
    let buffer = CircularRingBuffer::<_, 4>::new();

    buffer.write(10);
    assert!(buffer.read().is_some_and(|e| e == 10));
    buffer.write(20);
    assert!(buffer.read().is_some_and(|e| e == 20));
    buffer.write(30);
    assert!(buffer.read().is_some_and(|e| e == 30));
    buffer.write(40);
    assert!(buffer.read().is_some_and(|e| e == 40));
    buffer.write(50);
    assert!(buffer.read().is_some_and(|e| e == 50));
    buffer.write(60);
    assert!(buffer.read().is_some_and(|e| e == 60));
}
