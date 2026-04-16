use core::{
    ops::Deref,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::{lockfree::overwrite::RingBuffer, spin_loop, without_interrupts};

/// A ring buffer that allows for
/// - overwrite (still maintain push order)
/// - read re-play
/// - continuous write (buffer writeable while reading)
/// - MPSC
/// # Note
/// Partial read is not supported!
pub struct ContinuousRingBuffer<T, const N: usize> {
    buffers: [RingBuffer<T, N>; 3],
    rotation_state: AtomicUsize,
    writing: AtomicUsize,
    pivoting: AtomicBool,
}

impl<T, const N: usize> ContinuousRingBuffer<T, N> {
    #[cfg(not(feature = "loom_test"))]
    pub const fn new() -> Self {
        Self {
            buffers: [const { RingBuffer::new() }; 3],
            rotation_state: AtomicUsize::new(0),
            writing: AtomicUsize::new(0),
            pivoting: AtomicBool::new(false),
        }
    }

    #[cfg(feature = "loom_test")]
    pub fn new() -> Self {
        use core::array;

        Self {
            buffers: array::from_fn(|_| RingBuffer::new()),
            rotation_state: AtomicUsize::new(0),
            writing: AtomicUsize::new(0),
            pivoting: AtomicBool::new(false),
        }
    }

    pub fn write(&self, value: T) {
        while self.pivoting.load(Ordering::Acquire) {
            spin_loop();
        }

        without_interrupts(|| {
            self.writing.fetch_add(1, Ordering::Acquire);
            self.write_buffer().push(value);
            self.writing.fetch_sub(1, Ordering::Release);
        });
    }

    fn write_buffer(&self) -> &RingBuffer<T, N> {
        &self.buffers[self.rotation_state.load(Ordering::Acquire) % 3]
    }

    fn free_buffer(&self) -> &RingBuffer<T, N> {
        &self.buffers[self.rotation_state.load(Ordering::Acquire).wrapping_add(1) % 3]
    }

    fn read_buffer(&self) -> &RingBuffer<T, N> {
        &self.buffers[self.rotation_state.load(Ordering::Acquire).wrapping_add(2) % 3]
    }

    fn read_init(&self) -> (&RingBuffer<T, N>, &RingBuffer<T, N>) {
        without_interrupts(|| {
            self.pivoting.store(true, Ordering::Release); // Prevent new writes
            self.rotation_state.fetch_add(1, Ordering::Release); // Pivot the buffer

            while self.writing.load(Ordering::Acquire) != 0 {
                spin_loop();
            }

            self.pivoting.store(false, Ordering::Release); // Allow writing into the new buffer 
        });

        let read = self.read_buffer();
        let free = self.free_buffer();
        while let Some(value) = read.pop() {
            free.push(value);
        }
        (read, free)
    }

    pub fn read_cloned<'a>(&'a self) -> ContinuousBufferIterCloned<'a, T, N> {
        let (read_buffer, free_buffer) = self.read_init();
        ContinuousBufferIterCloned { read_buffer, free_buffer }
    }

    pub fn read_ref<'a>(&'a self) -> ContinuousBufferIterRef<'a, T, N> {
        let (read_buffer, free_buffer) = self.read_init();
        ContinuousBufferIterRef { read_buffer, free_buffer }
    }
}

impl<T, const N: usize> Default for ContinuousRingBuffer<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ContinuousBufferIterCloned<'a, T, const N: usize> {
    read_buffer: &'a RingBuffer<T, N>,
    free_buffer: &'a RingBuffer<T, N>,
}

impl<'a, T, const N: usize> Iterator for ContinuousBufferIterCloned<'a, T, N>
where
    T: Clone,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.free_buffer.pop()?;
        self.read_buffer.push(value.clone());
        Some(value)
    }
}

impl<'a, T, const N: usize> Drop for ContinuousBufferIterCloned<'a, T, N> {
    fn drop(&mut self) {
        while let Some(value) = self.free_buffer.pop() {
            self.read_buffer.push(value);
        }
    }
}

pub struct ContinuousBufferIterRef<'a, T, const N: usize> {
    read_buffer: &'a RingBuffer<T, N>,
    free_buffer: &'a RingBuffer<T, N>,
}

impl<'a, T, const N: usize> Iterator for ContinuousBufferIterRef<'a, T, N> {
    type Item = ContinousBufferRefGuard<'a, T, N>;

    fn next(&mut self) -> Option<Self::Item> {
        Some(ContinousBufferRefGuard { value: Some(self.free_buffer.pop()?), read_buffer: self.read_buffer })
    }
}

impl<'a, T, const N: usize> Drop for ContinuousBufferIterRef<'a, T, N> {
    fn drop(&mut self) {
        while let Some(value) = self.free_buffer.pop() {
            self.read_buffer.push(value);
        }
    }
}

pub struct ContinousBufferRefGuard<'a, T, const N: usize> {
    value: Option<T>,
    read_buffer: &'a RingBuffer<T, N>,
}

impl<'a, T, const N: usize> Deref for ContinousBufferRefGuard<'a, T, N> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value.as_ref().unwrap()
    }
}

impl<'a, T, const N: usize> Drop for ContinousBufferRefGuard<'a, T, N> {
    fn drop(&mut self) {
        self.read_buffer.push(self.value.take().unwrap());
    }
}

