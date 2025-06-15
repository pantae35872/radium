// WARNING!! WARNING!! .. GPT GENERATED !! GPT GENERATED !! GPT GENERATED !!

//! A bounded MPSC queue for non-Copy, non-Clone T in no_std.
//! Based on Dmitry Vyukov’s bounded queue algorithm.

use core::{
    cell::UnsafeCell,
    mem::{zeroed, MaybeUninit},
    ptr,
    sync::atomic::{AtomicUsize, Ordering},
};

/// A bounded MPSC queue with capacity N for T.
/// Multiple producers may call `push` concurrently; a single consumer may call `pop`.
pub struct SpinMPSC<T, const N: usize> {
    // Buffer of slots
    buffer: [Slot<T>; N],
    // Enqueue (tail) index
    tail: AtomicUsize,
    // Dequeue (head) index
    head: AtomicUsize,
}

// Each slot holds a sequence counter and storage for T
struct Slot<T> {
    sequence: AtomicUsize,
    data: UnsafeCell<MaybeUninit<T>>,
}

// Safety:
// - Producers only write into a slot after acquiring it via atomic logic.
// - Consumer only reads after the slot is ready, and then resets sequence.
// - T: Send to allow sending across threads.
unsafe impl<T: Send, const N: usize> Sync for SpinMPSC<T, N> {}
unsafe impl<T: Send, const N: usize> Send for SpinMPSC<T, N> {}

impl<T, const N: usize> SpinMPSC<T, N> {
    /// Create a new queue. N must be >= 2.
    pub const fn new() -> Self {
        // Ensure N >= 2 to avoid degenerate behavior.
        assert!(N >= 2, "Capacity N must be at least 2");

        const fn generate<T, const N: usize>(i: usize, mut v: [Slot<T>; N]) -> [Slot<T>; N] {
            if i == N {
                return v;
            }
            v[i] = Slot {
                sequence: AtomicUsize::new(i),
                data: UnsafeCell::new(MaybeUninit::uninit()),
            };
            generate(i + 1, v)
        }

        // Transmute to initialized array
        Self {
            buffer: generate(0, unsafe { zeroed() }),
            tail: AtomicUsize::new(0),
            head: AtomicUsize::new(0),
        }
    }

    /// Attempt to push a value. Returns Ok(()) if successful, or Err(value) if the queue is full.
    pub fn push(&self, value: T) -> Result<(), T> {
        loop {
            let head = self.head.load(Ordering::Acquire);
            let tail = self.tail.load(Ordering::Acquire);
            // Check if full: tail - head >= N
            if tail.wrapping_sub(head) >= N {
                return Err(value);
            }
            // Try to reserve slot at tail
            if self
                .tail
                .compare_exchange_weak(
                    tail,
                    tail.wrapping_add(1),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                // Successfully reserved position `pos`
                let pos = tail;
                let idx = pos % N;
                let slot = &self.buffer[idx];
                loop {
                    let seq = slot.sequence.load(Ordering::Acquire);
                    if seq == pos {
                        break;
                    }
                    core::hint::spin_loop();
                }
                unsafe {
                    let ptr = (*slot.data.get()).as_mut_ptr();
                    ptr::write(ptr, value);
                }
                // Publish by setting sequence = pos + 1
                slot.sequence.store(pos.wrapping_add(1), Ordering::Release);
                return Ok(());
            }
            // else CAS failed, retry
            core::hint::spin_loop();
        }
    }

    /// Attempt to pop a value. Returns Some(T) if successful, or None if empty.
    /// Only a single consumer must call pop.
    pub fn pop(&self) -> Option<T> {
        loop {
            let head = self.head.load(Ordering::Acquire);
            let tail = self.tail.load(Ordering::Acquire);
            // Empty if head >= tail
            if head >= tail {
                return None;
            }
            // Try to reserve slot at head
            if self
                .head
                .compare_exchange_weak(
                    head,
                    head.wrapping_add(1),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                let pos = head;
                let idx = pos % N;
                let slot = &self.buffer[idx];
                // Wait until slot.sequence == pos + 1 (data ready)
                loop {
                    let seq = slot.sequence.load(Ordering::Acquire);
                    if seq == pos.wrapping_add(1) {
                        break;
                    }
                    core::hint::spin_loop();
                }
                // Read the data
                let value = unsafe {
                    let ptr = (*slot.data.get()).as_ptr();
                    ptr::read(ptr)
                };
                // Mark slot free by setting sequence = pos + N
                slot.sequence.store(pos.wrapping_add(N), Ordering::Release);
                return Some(value);
            }
            // else CAS failed, retry
            core::hint::spin_loop();
        }
    }
}
