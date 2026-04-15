use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{AtomicUsize, Ordering},
};

/// A bounded MPSC queue with capacity N for T.
/// Multiple producers may call `push` concurrently; a single consumer may call `pop`.
pub struct Queue<T, const N: usize> {
    buffer: [Slot<T>; N],
    tail: AtomicUsize,
    head: AtomicUsize,
}

impl<T, const N: usize> Queue<T, N> {
    /// Create a new queue. N must be >= 2.
    pub const fn new() -> Self {
        // Ensure N >= 2 to avoid degenerate behavior.
        assert!(N >= 2, "Capacity N must be at least 2");

        let buffer: [Slot<T>; N] = {
            let mut arr: [MaybeUninit<Slot<T>>; N] = [const { MaybeUninit::uninit() }; N];
            let mut i = 0;

            while i < N {
                arr[i].write(Slot { sequence: AtomicUsize::new(i), data: UnsafeCell::new(MaybeUninit::uninit()) });
                i += 1;
            }

            // SAFETY: It is initialized above
            unsafe { MaybeUninit::array_assume_init(arr) }
        };

        Self { buffer, tail: AtomicUsize::new(0), head: AtomicUsize::new(0) }
    }

    /// Attempt to push a value. Returns Ok(()) if successful, or Err(value) if the queue is full.
    pub fn push(&self, value: T) -> Result<(), T> {
        let Ok(pos) = self.tail.try_update(Ordering::AcqRel, Ordering::Acquire, |tail| {
            let head = self.head.load(Ordering::Acquire);
            if tail.wrapping_sub(head) >= N {
                return None;
            }
            Some(tail.wrapping_add(1))
        }) else {
            return Err(value);
        };

        let slot = &self.buffer[pos % N];
        while slot.sequence.load(Ordering::Acquire) != pos {
            core::hint::spin_loop();
        }

        unsafe {
            slot.data.get().write(MaybeUninit::new(value));
        }

        slot.sequence.store(pos.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    pub fn peek(&self) -> Option<&T> {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        if head >= tail {
            return None;
        }
        let pos = head;
        let slot = &self.buffer[pos % N];
        let seq = slot.sequence.load(Ordering::Acquire);

        if seq != pos.wrapping_add(1) {
            return None;
        }

        // Safe: writer has published T at this slot, and single consumer holds off pop until consuming.
        Some(unsafe { (*slot.data.get()).assume_init_ref() })
    }

    /// Attempt to pop a value. Returns Some(T) if successful, or None if empty.
    /// Only a single consumer must call pop.
    pub fn pop(&self) -> Option<T> {
        let Ok(pos) = self.head.try_update(Ordering::AcqRel, Ordering::Acquire, |head| {
            let tail = self.tail.load(Ordering::Acquire);

            if head >= tail {
                return None;
            }
            Some(head.wrapping_add(1))
        }) else {
            return None;
        };

        let slot = &self.buffer[pos % N];

        // Wait until slot.sequence == pos + 1 (data ready)
        while slot.sequence.load(Ordering::Acquire) != pos.wrapping_add(1) {
            core::hint::spin_loop();
        }
        // Read the data
        let value = unsafe { (*slot.data.get()).assume_init_read() };
        slot.sequence.store(pos.wrapping_add(N), Ordering::Release);
        Some(value)
    }
}

// SAFETY:
// - Producers only write into a slot after acquiring it via atomic logic.
// - Consumer only reads after the slot is ready, and then resets sequence.
// - T: Send to allow sending across threads.
unsafe impl<T: Send, const N: usize> Sync for Queue<T, N> {}
unsafe impl<T: Send, const N: usize> Send for Queue<T, N> {}

impl<T, const N: usize> Default for Queue<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

// Each slot holds a sequence counter and storage for T
struct Slot<T> {
    data: UnsafeCell<MaybeUninit<T>>,
    sequence: AtomicUsize,
}
