#![no_std]
#![feature(maybe_uninit_array_assume_init)]
#![feature(sync_unsafe_cell)]

use core::{arch::asm, cell::SyncUnsafeCell, fmt::Debug};

#[cfg(feature = "interrupt_safe")]
use pager::registers::RFlags;

extern crate alloc;

pub mod lockfree;
pub mod singlethreaded;

#[repr(transparent)]
pub struct VolatileCell<T> {
    value: SyncUnsafeCell<T>,
}

impl<T: Copy> VolatileCell<T> {
    #[inline]
    pub fn get(&self) -> T {
        unsafe { core::ptr::read_volatile(self.value.get()) }
    }

    #[inline]
    pub fn set(&self, value: T) {
        unsafe { core::ptr::write_volatile(self.value.get(), value) }
    }
}

impl<T: Copy + Debug> Debug for VolatileCell<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.get())
    }
}

#[inline(always)]
fn spin_loop() {
    #[cfg(not(feature = "loom_test"))]
    core::hint::spin_loop();

    #[cfg(feature = "loom_test")]
    loom::thread::yield_now();
}

#[inline(always)]
pub fn disable() {
    // SAFETY: Enabling and Disabling interrupt is considered safe in kernel context
    unsafe { asm!("cli", options(nomem, nostack)) }
}

#[inline(always)]
pub fn enable() {
    // SAFETY: Enabling and Disabling interrupt is considered safe in kernel context
    unsafe { asm!("sti", options(nomem, nostack)) }
}

#[inline(always)]
pub fn without_interrupts<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    #[cfg(feature = "interrupt_safe")]
    {
        let was_enable = RFlags::read().contains(RFlags::InterruptEnable);
        if was_enable {
            disable();
        }

        let ret = f();

        if was_enable {
            enable();
        }
        ret
    }
    #[cfg(not(feature = "interrupt_safe"))]
    f()
}
