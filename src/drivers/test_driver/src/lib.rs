#![no_std]

pub struct AABB {
    a: u64,
    b: u64,
    c: u64,
    d: u64,
}

static mut TEST_GLOBAL: u64 = 10;

static mut AABB: AABB = AABB {
    a: 40,
    b: 30,
    c: 20,
    d: 10,
};

#[inline(never)]
pub(crate) fn add(a: u64, b: u64) -> u64 {
    a + b
}

#[unsafe(no_mangle)]
pub extern "C" fn start(a: u64, b: u64) -> u64 {
    unsafe { external_func() };
    unsafe { add(a, b) + TEST_GLOBAL }
}

#[unsafe(no_mangle)]
pub extern "C" fn aabb(a: u64, b: u64) {
    unsafe {
        AABB.a += a;
        AABB.b += b;
        AABB.c += a * 2;
        AABB.d += b * 2;
    }
}

#[unsafe(no_mangle)]
pub fn change(a: u64) {
    unsafe {
        TEST_GLOBAL += a;
    }
}

use core::panic::PanicInfo;

unsafe extern "Rust" {
    fn kpanic(info: &PanicInfo) -> !;
}

unsafe extern "Rust" {
    fn external_func();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe { kpanic(info) }
}
