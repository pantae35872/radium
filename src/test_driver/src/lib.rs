#![no_std]

#[unsafe(no_mangle)]
pub static mut TEST_GLOBAL: u64 = 0;

#[inline(never)]
pub(crate) fn add(a: u64, b: u64) -> u64 {
    a + b
}

#[unsafe(no_mangle)]
pub fn start(a: u64, b: u64) -> u64 {
    unsafe { add(a, b) + TEST_GLOBAL }
}

#[unsafe(no_mangle)]
pub fn change(a: u64) {
    unsafe {
        TEST_GLOBAL += a;
    }
}

#[cfg(not(test))]
mod panic_handler {
    use core::panic::PanicInfo;

    #[panic_handler]
    fn panic(_info: &PanicInfo) -> ! {
        loop {}
    }
}
