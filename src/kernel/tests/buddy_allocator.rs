#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![feature(pointer_is_aligned_to)]
#![reexport_test_harness_main = "test_main"]
#![test_runner(radium::test_runner)]

extern crate alloc;
extern crate radium;

use alloc::vec::Vec;
use bootbridge::RawBootBridge;
use radium::smp::CTX;

#[unsafe(no_mangle)]
pub extern "C" fn start(boot_bridge: *mut RawBootBridge) -> ! {
    radium::init(boot_bridge, test_main);
}

#[test_case]
fn simple_alloc() {
    let sizes = [16, 32, 16, 32, 8, 8, 16, 128, 1024];
    let mut allocations = Vec::new();
    let mut allocation_ranges = Vec::new();

    for &size in sizes.iter() {
        let ptr = CTX.lock().buddy_allocator().allocate(size);
        assert!(ptr.is_some(), "Allocation failed for size: {}", size);
        let ptr = ptr.unwrap();
        assert!(ptr.is_aligned_to(size));

        let start = ptr as usize;
        let end = start + size - 1;

        allocation_ranges.push((start, end));
        allocations.push((ptr, size));
    }

    let large_size = CTX.lock().buddy_allocator().max_mem() * 8;
    let ptr = CTX.lock().buddy_allocator().allocate(large_size);
    assert!(
        ptr.is_none(),
        "Allocation should fail for size: {}",
        large_size
    );

    for i in 0..allocation_ranges.len() {
        for j in i + 1..allocation_ranges.len() {
            let (start_i, end_i) = allocation_ranges[i];
            let (start_j, end_j) = allocation_ranges[j];

            assert!(
                end_i < start_j || end_j < start_i,
                "Memory overlap detected between allocations: ({}, {}) and ({}, {})",
                start_i,
                end_i,
                start_j,
                end_j
            );
        }
    }
}

#[test_case]
fn alloc_free() {
    let sizes = [8, 16, 32, 64, 128];
    let mut allocations = Vec::new();

    for &size in sizes.iter() {
        let ptr = CTX.lock().buddy_allocator().allocate(size);
        assert!(ptr.is_some(), "Allocation failed for size: {}", size);
        allocations.push((ptr.unwrap(), size));
    }

    for &(ptr, size) in &allocations {
        CTX.lock().buddy_allocator().dealloc(ptr, size);
    }

    for (i, &size) in sizes.iter().enumerate() {
        let ptr = CTX.lock().buddy_allocator().allocate(size);
        assert!(ptr.is_some(), "Reallocation failed for size: {}", size);
        assert_eq!(
            ptr.unwrap(),
            allocations[i].0,
            "Reallocated pointer does not match original"
        );
    }
}
