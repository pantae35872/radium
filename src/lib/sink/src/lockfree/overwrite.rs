#[cfg(not(feature = "loom_test"))]
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicUsize, Ordering},
};

#[cfg(feature = "loom_test")]
use loom::{
    cell::UnsafeCell,
    sync::atomic::{AtomicUsize, Ordering},
};

use core::mem::MaybeUninit;

use crate::{spin_loop, without_interrupts};

pub mod continuous;

/// A bounded MPSC queue with overwrite-on-full semantics.
///
/// Ordering guarantee:
/// - items that are observed by the consumer are observed in the exact
///   logical write order
/// - older items may be dropped when the queue overflows
pub struct RingBuffer<T, const N: usize> {
    buffer: [Slot<T>; N],
    tail: AtomicUsize,
    head: AtomicUsize,
}

impl<T, const N: usize> RingBuffer<T, N> {
    #[cfg(not(feature = "loom_test"))]
    pub const fn new() -> Self {
        assert!(N >= 2, "Capacity N must be at least 2");

        let buffer: [Slot<T>; N] = {
            let mut arr: [MaybeUninit<Slot<T>>; N] = [const { MaybeUninit::uninit() }; N];
            let mut i = 0;

            while i < N {
                arr[i].write(Slot { sequence: AtomicUsize::new(i), data: UnsafeCell::new(MaybeUninit::uninit()) });
                i += 1;
            }

            unsafe { MaybeUninit::array_assume_init(arr) }
        };

        Self { buffer, tail: AtomicUsize::new(0), head: AtomicUsize::new(0) }
    }

    #[cfg(feature = "loom_test")]
    pub fn new() -> Self {
        assert!(N >= 2, "Capacity N must be at least 2");

        let buffer: [Slot<T>; N] = {
            let mut arr: [MaybeUninit<Slot<T>>; N] = [const { MaybeUninit::uninit() }; N];
            let mut i = 0;

            while i < N {
                arr[i].write(Slot { sequence: AtomicUsize::new(i), data: UnsafeCell::new(MaybeUninit::uninit()) });
                i += 1;
            }

            unsafe { MaybeUninit::array_assume_init(arr) }
        };

        Self { buffer, tail: AtomicUsize::new(0), head: AtomicUsize::new(0) }
    }

    #[inline]
    fn release_range(&self, start: usize, end: usize) {
        let mut i = start;
        while i < end {
            self.buffer[i % N].sequence.store(i.wrapping_add(N), Ordering::Release);
            i = i.wrapping_add(1);
        }
    }

    /// Push with overwrite-on-full semantics.
    ///
    /// Returns `Err(value)` only if this producer's reservation was already
    /// overtaken and dropped before it managed to publish.
    pub fn push(&self, value: T) {
        'reserve: loop {
            let pos = self.tail.fetch_add(1, Ordering::AcqRel);
            let slot = &self.buffer[pos % N];

            loop {
                let head = self.head.load(Ordering::Acquire);
                if head > pos {
                    continue 'reserve;
                }

                let desired_head = pos.saturating_add(1).saturating_sub(N);
                if head < desired_head {
                    if self.head.compare_exchange(head, desired_head, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                        self.release_range(head, desired_head);
                    }
                    continue;
                }

                // Exact write order: only publish when our slot is the one for `pos`.
                if slot.sequence.load(Ordering::Acquire) != pos {
                    spin_loop();
                    continue;
                }

                without_interrupts(|| {
                    unsafe {
                        #[cfg(not(feature = "loom_test"))]
                        (*slot.data.get()).write(value);

                        #[cfg(feature = "loom_test")]
                        slot.data.get_mut().deref().write(value);
                    }

                    slot.sequence.store(pos.wrapping_add(1), Ordering::Release);
                });

                return;
            }
        }
    }

    /// Single-consumer pop. Returns items in exact logical order.
    pub fn pop(&self) -> Option<T> {
        loop {
            let head = self.head.load(Ordering::Acquire);
            let tail = self.tail.load(Ordering::Acquire);

            if head >= tail {
                return None;
            }

            let slot = &self.buffer[head % N];
            let seq = slot.sequence.load(Ordering::Acquire);

            if seq == head.wrapping_add(1) {
                // Claim the item before reading so no producer can overwrite it.
                if self.head.compare_exchange(head, head.wrapping_add(1), Ordering::AcqRel, Ordering::Acquire).is_err()
                {
                    continue;
                }

                return Some(without_interrupts(|| {
                    let value = unsafe {
                        #[cfg(not(feature = "loom_test"))]
                        let value = (*slot.data.get()).assume_init_read();

                        #[cfg(feature = "loom_test")]
                        let value = slot.data.get_mut().deref().assume_init_read();

                        value
                    };

                    slot.sequence.store(head.wrapping_add(N), Ordering::Release);
                    value
                }));
            }

            // Someone may have advanced head for overwrite; re-check.
            if self.head.load(Ordering::Acquire) != head {
                continue;
            }

            spin_loop();
        }
    }
}

unsafe impl<T: Send, const N: usize> Sync for RingBuffer<T, N> {}
unsafe impl<T: Send, const N: usize> Send for RingBuffer<T, N> {}

impl<T, const N: usize> Default for RingBuffer<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

struct Slot<T> {
    data: UnsafeCell<MaybeUninit<T>>,
    sequence: AtomicUsize,
}

#[cfg(all(feature = "loom_test", test))]
mod test_loom {
    use alloc::vec::Vec;
    use loom::{sync::Arc, thread};

    use crate::lockfree::overwrite::RingBuffer;

    #[derive(Debug)]
    struct Data {
        thread: usize,
        count: usize,
    }

    fn loom_test<const P: usize, const S: usize>() {
        let data = Arc::new(RingBuffer::<Data, S>::new());
        let mut handles = Vec::new();

        for thread in 0..P {
            let data = data.clone();
            handles.push(thread::spawn(move || {
                for count in 1..4 {
                    data.push(Data { thread, count });
                }
            }));
        }

        let mut thread_counts = [0; P];
        while let Some(pop) = data.pop() {
            match thread_counts[pop.thread].cmp(&pop.count) {
                core::cmp::Ordering::Greater => panic!("Thread data got reordered"),
                core::cmp::Ordering::Equal => panic!("Thread send duplicated data!"),
                core::cmp::Ordering::Less => {
                    thread_counts[pop.thread] = pop.count;
                }
            }
        }

        for handle in handles {
            handle.join().unwrap();
        }

        while let Some(pop) = data.pop() {
            match thread_counts[pop.thread].cmp(&pop.count) {
                core::cmp::Ordering::Greater => panic!("Thread data got reordered"),
                core::cmp::Ordering::Equal => panic!("Thread send duplicated data!"),
                core::cmp::Ordering::Less => {
                    thread_counts[pop.thread] = pop.count;
                }
            }
        }
    }

    #[test]
    fn loom_2_producer() {
        loom::model(|| {
            loom_test::<2, 5>();
        });
    }

    #[test]
    fn loom_3_producer() {
        loom::model(|| {
            loom_test::<3, 8>();
        });
    }
}

#[cfg(all(not(feature = "loom_test"), test))]
mod test {
    use crate::lockfree::overwrite::RingBuffer;

    #[test]
    pub fn read_write() {
        let buffer = RingBuffer::<_, 5>::new();
        buffer.push(30);
        buffer.push(20);
        assert!(buffer.pop().is_some_and(|e| e == 30));
        assert!(buffer.pop().is_some_and(|e| e == 20));
        assert!(buffer.pop().is_none());
        buffer.push(40);
        buffer.push(50);
        assert!(buffer.pop().is_some_and(|e| e == 40));
        assert!(buffer.pop().is_some_and(|e| e == 50));
        assert!(buffer.pop().is_none());
    }

    #[test]
    pub fn read_write_overwrite() {
        let buffer = RingBuffer::<_, 5>::new();
        buffer.push(30);
        buffer.push(20);
        buffer.push(40);
        buffer.push(50);
        buffer.push(60);
        buffer.push(70);
        assert!(buffer.pop().is_some_and(|e| e == 20));
        assert!(buffer.pop().is_some_and(|e| e == 40));
        assert!(buffer.pop().is_some_and(|e| e == 50));
        assert!(buffer.pop().is_some_and(|e| e == 60));
        assert!(buffer.pop().is_some_and(|e| e == 70));
        assert!(buffer.pop().is_none());
    }

    #[test]
    pub fn interleaved_pop_push() {
        let buffer = RingBuffer::<_, 5>::new();

        buffer.push(10);
        buffer.push(20);

        assert!(buffer.pop().is_some_and(|e| e == 10));

        buffer.push(30);
        buffer.push(40);
        buffer.push(50);

        assert!(buffer.pop().is_some_and(|e| e == 20));
        assert!(buffer.pop().is_some_and(|e| e == 30));
        assert!(buffer.pop().is_some_and(|e| e == 40));
        assert!(buffer.pop().is_some_and(|e| e == 50));
        assert!(buffer.pop().is_none());
    }

    #[test]
    pub fn sequential_pop_push() {
        let buffer = RingBuffer::<_, 4>::new();

        buffer.push(10);
        assert!(buffer.pop().is_some_and(|e| e == 10));
        buffer.push(20);
        assert!(buffer.pop().is_some_and(|e| e == 20));
        buffer.push(30);
        assert!(buffer.pop().is_some_and(|e| e == 30));
        buffer.push(40);
        assert!(buffer.pop().is_some_and(|e| e == 40));
        buffer.push(50);
        assert!(buffer.pop().is_some_and(|e| e == 50));
        buffer.push(60);
        assert!(buffer.pop().is_some_and(|e| e == 60));
    }
}
