#![no_std]
#![feature(maybe_uninit_array_assume_init)]
#![feature(sync_unsafe_cell)]

use core::{arch::asm, cell::SyncUnsafeCell, fmt::Debug};

use pager::registers::RFlags;

pub mod lockfree;

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
