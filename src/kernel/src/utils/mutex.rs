use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicUsize, Ordering},
};

use pager::address::VirtAddr;

use crate::{
    scheduler::{futex_wait, futex_wake},
    smp::{cpu_local, cpu_local_avaiable},
};

pub struct Mutex<T> {
    lock: AtomicUsize,
    data: UnsafeCell<T>,
}

unsafe impl<T> Sync for Mutex<T> {}

pub struct MutexGuard<'a, T> {
    lock: &'a AtomicUsize,
    data: *mut T,
}

impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            lock: AtomicUsize::new(0),
            data: UnsafeCell::new(value),
        }
    }

    pub unsafe fn force_unlock(&self) {
        self.lock.store(0, Ordering::SeqCst);
    }

    pub fn lock(&self) -> MutexGuard<'_, T> {
        while self
            .lock
            .compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            // TODO: Due to futex is not yet stable enough to use, threads can be dead lock or lost
            // permanently
            if cpu_local_avaiable() && !cpu_local().is_in_isr {
                unsafe { futex_wait(VirtAddr::new(&self.lock as *const AtomicUsize as u64), 1) };
            }
        }
        return MutexGuard {
            lock: &self.lock,
            data: unsafe { &mut *self.data.get() },
        };
    }
}

impl<'a, T> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.store(0, Ordering::SeqCst);

        // TODO: Due to futex is not yet stable enough to use, threads can be dead lock or lost
        // permanently
        if cpu_local_avaiable() && !cpu_local().is_in_isr {
            unsafe { futex_wake(VirtAddr::new(self.lock as *const AtomicUsize as u64)) };
        }
    }
}

impl<'a, T> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.data }
    }
}

impl<'a, T> Deref for MutexGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.data }
    }
}
