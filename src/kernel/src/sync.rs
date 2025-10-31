use core::{cell::SyncUnsafeCell, fmt::Debug};

pub mod circular_ring_buffer;
pub mod mutex;
pub mod spin_mpsc;

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
