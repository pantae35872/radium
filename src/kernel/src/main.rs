#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(radium::test_runner)]
#![reexport_test_harness_main = "test_main"]
#![feature(core_intrinsics)]
#![feature(pointer_is_aligned_to)]
#![recursion_limit = "512"]
#![allow(internal_features)]
#![allow(clippy::fn_to_numeric_cast)]

extern crate alloc;
extern crate core;
extern crate lazy_static;
extern crate radium;
extern crate spin;

use core::alloc::Layout;
use core::intrinsics::compare_bytes;

#[unsafe(no_mangle)]
pub extern "C" fn start(boot_bridge: *mut bootbridge::RawBootBridge) -> ! {
    radium::init(boot_bridge, compare_bytes_bug);
}

fn compare_bytes_bug() {
    const SIZE: usize = 88; // Can be any size

    // Allocate 8-byte aligned memory
    let layout_aligned = Layout::from_size_align(SIZE, 8).unwrap();
    let aligned_ptr = unsafe { alloc::alloc::alloc(layout_aligned) };

    // Allocate 1-byte aligned memory (guaranteed unaligned for >1)
    let layout_unaligned = Layout::from_size_align(SIZE + 7, 1).unwrap(); // +7 to offset
    let raw_unaligned = unsafe { alloc::alloc::alloc(layout_unaligned) };
    let offset = raw_unaligned.align_offset(8);
    let unaligned_ptr = unsafe { raw_unaligned.add((offset + 1) % 8) }; // misalign on purpose

    // Fill aligned_ptr with data
    for i in 0..SIZE {
        unsafe { *aligned_ptr.add(i) = i as u8 };
    }

    unsafe {
        core::ptr::copy_nonoverlapping(aligned_ptr, unaligned_ptr, SIZE);
    }

    assert!(unaligned_ptr.is_aligned_to(1) && !unaligned_ptr.is_aligned_to(2));
    assert!(aligned_ptr.is_aligned_to(8));

    unsafe { compare_bytes(aligned_ptr, unaligned_ptr, SIZE) };
}
