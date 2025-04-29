use core::{cell::SyncUnsafeCell, fmt::Debug};

pub mod buffer_reader;
pub mod circular_ring_buffer;

#[macro_export]
macro_rules! inline_if {
    ($condition:expr, $true_expr:expr, $false_expr:expr) => {
        if $condition {
            $true_expr
        } else {
            $false_expr
        }
    };
}

pub trait NumberUtils<T> {
    fn prev_power_of_two(self) -> T;
}

impl NumberUtils<usize> for usize {
    fn prev_power_of_two(self) -> usize {
        1 << (usize::BITS as usize - self.leading_zeros() as usize - 1)
    }
}

pub fn floorf64(x: f64) -> f64 {
    let integer_part = x as i64;

    if x >= 0.0 || x == integer_part as f64 {
        integer_part as f64
    } else {
        (integer_part - 1) as f64
    }
}

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

#[macro_export]
macro_rules! defer {
    ($body:expr) => {
        let _defer = {
            use $crate::utils::Defer;
            Defer::new(|| $body)
        };
    };
}

pub struct Defer<F: FnOnce() -> T, T> {
    func: Option<F>,
}

impl<F: FnOnce() -> T, T> Defer<F, T> {
    pub fn new(func: F) -> Self {
        Defer { func: Some(func) }
    }
}

impl<F: FnOnce() -> T, T> Drop for Defer<F, T> {
    fn drop(&mut self) {
        if let Some(func) = self.func.take() {
            func();
        }
    }
}
