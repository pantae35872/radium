// Derived from https://github.com/tchajed/futex-tutorial/blob/main/mutex_better.c

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

const UNLOCKED: usize = 0;
const LOCKED_NO_WAIT: usize = 1;
const LOCKED_WAIT: usize = 2;

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

    /// # Safety
    /// Force unlocking an mutex is unsafe, can causes data corruption and races
    pub unsafe fn force_unlock(&self) {
        self.lock.store(0, Ordering::SeqCst);
    }

    pub fn lock(&self) -> MutexGuard<'_, T> {
        // FIXME: Use some one core mutex implementation because the situation where cpu local is
        // not avaiable either the early initializatoin of the kernel or other cores
        if !cpu_local_avaiable() {
            return MutexGuard {
                lock: &self.lock,
                data: unsafe { &mut *self.data.get() },
            };
        }
        assert!(
            !cpu_local().is_in_isr,
            "Futex Mutex can't be use in interrupt context"
        );
        let mut c = UNLOCKED;
        if self
            .lock
            .compare_exchange_weak(c, LOCKED_NO_WAIT, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            return MutexGuard {
                lock: &self.lock,
                data: unsafe { &mut *self.data.get() },
            };
        }
        loop {
            if c == LOCKED_WAIT
                || self
                    .lock
                    .compare_exchange(
                        LOCKED_NO_WAIT,
                        LOCKED_WAIT,
                        Ordering::Acquire,
                        Ordering::Relaxed,
                    )
                    .err()
                    .is_some()
            {
                unsafe {
                    futex_wait(
                        VirtAddr::new(&self.lock as *const AtomicUsize as u64),
                        LOCKED_WAIT,
                    )
                };
            }

            c = UNLOCKED;

            if self
                .lock
                .compare_exchange_weak(c, LOCKED_WAIT, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }

        MutexGuard {
            lock: &self.lock,
            data: unsafe { &mut *self.data.get() },
        }
    }
}

impl<'a, T> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        if !cpu_local_avaiable() {
            return;
        }
        assert!(!cpu_local().is_in_isr);
        if self.lock.fetch_sub(1, Ordering::Release) != LOCKED_NO_WAIT {
            self.lock.store(UNLOCKED, Ordering::Release);
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
