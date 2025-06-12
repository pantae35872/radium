#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(radium::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;
extern crate core;
extern crate lazy_static;
extern crate radium;
extern crate spin;

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::vec::Vec;
use bootbridge::RawBootBridge;
use pager::address::VirtAddr;
use radium::driver::uefi_runtime::uefi_runtime;
use radium::logger::LOGGER;
use radium::scheduler::{futex_wait, futex_wake, sleep};
use radium::smp::cpu_local;
use radium::{hlt_loop, print, println, serial_print, serial_println};
use sentinel::log;

// TODO: Implements acpi to get io apic
// TODO: Use ahci interrupt (needs io apic) with waker
// TODO: Implements waker based async mutex
// TODO: Impelemnts kernel services executor

static TEST_MUTEX: DumbMutex<Vec<usize>> = DumbMutex::new(Vec::new());

struct DumbMutex<T> {
    lock: AtomicUsize,
    data: UnsafeCell<T>,
}

unsafe impl<T> Sync for DumbMutex<T> {}

struct MutexGuard<'a, T> {
    lock: &'a AtomicUsize,
    data: *mut T,
}

impl<T> DumbMutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            lock: AtomicUsize::new(0),
            data: UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, T> {
        while self
            .lock
            .compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            unsafe { futex_wait(VirtAddr::new(&self.lock as *const AtomicUsize as u64), 1) };
        }
        return MutexGuard {
            lock: &self.lock,
            data: unsafe { &mut *self.data.get() },
        };
    }
}

impl<'a, T> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.store(0, Ordering::Release);
        unsafe {
            futex_wake(VirtAddr::new(self.lock as *const AtomicUsize as u64), 1);
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

#[unsafe(no_mangle)]
pub extern "C" fn start(boot_bridge: *mut RawBootBridge) -> ! {
    radium::init(boot_bridge, kmain_thread);
}

fn kmain_thread() {
    println!("Hello, world!!!, from kmain thread");
    log!(Info, "Time {:?}", uefi_runtime().lock().get_time());
    cpu_local().local_scheduler().spawn(|| {
        for i in 0..64 {
            let mut mutex = TEST_MUTEX.lock();
            println!(
                "hello from thread: {}, {i}, {:?}",
                cpu_local().current_thread_id(),
                mutex.pop()
            );
            println!("{:?}", uefi_runtime().lock().get_time());
            sleep(1000);
        }
    });
    cpu_local().local_scheduler().spawn(|| {
        for i in 0..64 {
            serial_println!(
                "hello from thread: {}, {i}",
                cpu_local().current_thread_id()
            );
            sleep(50);
            TEST_MUTEX.lock().push(i * 10);
        }
    });

    sleep(5000);

    cpu_local().local_scheduler().spawn(|| {
        log!(
            Debug,
            "this should be thread 2, current tid {}",
            cpu_local().current_thread_id()
        );
    });

    LOGGER.flush_all(&[|s| serial_print!("{s}"), |s| print!("{s}")]);

    #[cfg(test)]
    test_main();

    hlt_loop();
}
