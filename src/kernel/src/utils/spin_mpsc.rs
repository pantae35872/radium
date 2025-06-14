use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::interrupt;

pub struct SpinMPSC<T, const N: usize> {
    buffer: [Slot<T>; N],
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl<T, const N: usize> SpinMPSC<T, N> {
    pub const fn new() -> Self {
        Self {
            buffer: [const { Slot::new() }; N],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    pub fn push(&self, value: T) -> Option<()> {
        let mut tail = self.head.load(Ordering::Relaxed);

        loop {
            let head = self.head.load(Ordering::Acquire);
            if tail.wrapping_sub(head) >= N {
                return None;
            }

            match self.tail.compare_exchange(
                tail,
                tail.wrapping_add(1),
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    let index = tail % N;
                    self.buffer[index].write(value);
                    break Some(());
                }
                Err(updated) => tail = updated,
            };
        }
    }

    pub fn pop(&self) -> Option<T> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);

        if head == tail {
            return None;
        }

        let index = head % N;
        let value = self.buffer[index].take();
        self.head.store(head.wrapping_add(1), Ordering::Release);
        value
    }
}

impl<T, const N: usize> Default for SpinMPSC<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl<T: Send, const N: usize> Sync for SpinMPSC<T, N> {}
unsafe impl<T: Send, const N: usize> Send for SpinMPSC<T, N> {}

struct Slot<T> {
    state: AtomicUsize, // 0 = empty, 1 = writing, 2 = full
    data: UnsafeCell<Option<T>>,
}

impl<T> Slot<T> {
    const fn new() -> Self {
        Self {
            state: AtomicUsize::new(0),
            data: UnsafeCell::new(None),
        }
    }

    fn write(&self, value: T) {
        interrupt::without_interrupts(|| loop {
            match self
                .state
                .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(..) => {
                    let data = unsafe { &mut *self.data.get() };
                    *data = Some(value);

                    // This point is where interrupts may occurs, and causing a deadlock

                    self.state.store(2, Ordering::Release);
                    break;
                }
                Err(state) => match state {
                    1 => continue,
                    2 => {
                        let data = unsafe { &mut *self.data.get() };
                        *data = Some(value);
                        break;
                    }
                    _ => panic!("Invalid state in slot"),
                },
            }
        });
    }

    fn take(&self) -> Option<T> {
        interrupt::without_interrupts(|| loop {
            match self
                .state
                .compare_exchange(2, 1, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(..) => {
                    let data = unsafe { &mut *self.data.get() };
                    let result = data.take();

                    self.state.store(0, Ordering::Release);

                    return result;
                }
                Err(state) => match state {
                    1 => continue,
                    0 => break None,
                    _ => panic!("Invalid state in slot"),
                },
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    pub fn test_single_threaded_push_pop() {
        const CAP: usize = 4;
        let q: SpinMPSC<u32, CAP> = SpinMPSC::new();

        // Fill the queue
        assert_eq!(q.push(1), Some(()));
        assert_eq!(q.push(2), Some(()));
        assert_eq!(q.push(3), Some(()));
        assert_eq!(q.push(4), Some(()));
        assert_eq!(q.push(5), None); // Should fail (queue full)

        // Pop all elements
        assert_eq!(q.pop(), Some(1));
        assert_eq!(q.pop(), Some(2));
        assert_eq!(q.pop(), Some(3));
        assert_eq!(q.pop(), Some(4));
        assert_eq!(q.pop(), None); // Should be empty now

        // Push again to check wraparound
        assert_eq!(q.push(10), Some(()));
        assert_eq!(q.push(11), Some(()));
        assert_eq!(q.pop(), Some(10));
        assert_eq!(q.pop(), Some(11));
        assert_eq!(q.pop(), None);
    }

    //#[test_case]
    //pub fn read_write() {
    //    let buffer = SpinMPSC::<_, 5>::new();
    //    assert!(buffer.push(30).is_some());
    //    assert!(buffer.push(20).is_some());
    //    assert!(buffer.pop().is_some_and(|e| e == 30));
    //    assert!(buffer.pop().is_some_and(|e| e == 20));
    //    assert!(buffer.pop().is_none());
    //    assert!(buffer.push(40).is_some());
    //    assert!(buffer.push(50).is_some());
    //    assert!(buffer.pop().is_some_and(|e| e == 40));
    //    assert!(buffer.pop().is_some_and(|e| e == 50));
    //    assert!(buffer.pop().is_none());
    //}

    //#[test_case]
    //pub fn read_write_err() {
    //    let buffer = SpinMPSC::<_, 6>::new();
    //    assert!(buffer.push(30).is_some());
    //    assert!(buffer.push(20).is_some());
    //    assert!(buffer.push(40).is_some());
    //    assert!(buffer.push(50).is_some());
    //    assert!(buffer.push(60).is_some());
    //    assert!(buffer.push(70).is_none());
    //    assert!(buffer.pop().is_some_and(|e| e == 30));
    //    assert!(buffer.pop().is_some_and(|e| e == 20));
    //    assert!(buffer.pop().is_some_and(|e| e == 40));
    //    assert!(buffer.pop().is_some_and(|e| e == 50));
    //    assert!(buffer.pop().is_some_and(|e| e == 60));
    //    assert!(buffer.pop().is_none());
    //    assert!(buffer.pop().is_none());
    //    assert!(buffer.pop().is_none());
    //    assert!(buffer.pop().is_none());
    //    assert!(buffer.pop().is_none());

    //    assert!(buffer.push(30).is_some());
    //    assert!(buffer.push(20).is_some());
    //    assert!(buffer.push(40).is_some());
    //    assert!(buffer.push(50).is_some());
    //    assert!(buffer.push(60).is_some());
    //    assert!(buffer.push(70).is_none());
    //    assert!(buffer.pop().is_some_and(|e| e == 30));
    //    assert!(buffer.pop().is_some_and(|e| e == 20));
    //    assert!(buffer.pop().is_some_and(|e| e == 40));
    //    assert!(buffer.pop().is_some_and(|e| e == 50));
    //    assert!(buffer.pop().is_some_and(|e| e == 60));
    //}

    //#[test_case]
    //pub fn interleaved_read_write() {
    //    let buffer = SpinMPSC::<_, 5>::new();

    //    assert!(buffer.push(10).is_some());
    //    assert!(buffer.push(20).is_some());

    //    assert!(buffer.pop().is_some_and(|e| e == 10));

    //    assert!(buffer.push(30).is_some());
    //    assert!(buffer.push(40).is_some());
    //    assert!(buffer.push(50).is_some());

    //    assert!(buffer.pop().is_some_and(|e| e == 20));
    //    assert!(buffer.pop().is_some_and(|e| e == 30));
    //    assert!(buffer.pop().is_some_and(|e| e == 40));
    //    assert!(buffer.pop().is_some_and(|e| e == 50));
    //    assert!(buffer.pop().is_none());
    //}

    //#[test_case]
    //pub fn sequential_read_write() {
    //    let buffer = SpinMPSC::<_, 4>::new();

    //    assert!(buffer.push(10).is_some());
    //    assert!(buffer.pop().is_some_and(|e| e == 10));
    //    assert!(buffer.push(20).is_some());
    //    assert!(buffer.pop().is_some_and(|e| e == 20));
    //    assert!(buffer.push(30).is_some());
    //    assert!(buffer.pop().is_some_and(|e| e == 30));
    //    assert!(buffer.push(40).is_some());
    //    assert!(buffer.pop().is_some_and(|e| e == 40));
    //    assert!(buffer.push(50).is_some());
    //    assert!(buffer.pop().is_some_and(|e| e == 50));
    //    assert!(buffer.push(60).is_some());
    //    assert!(buffer.pop().is_some_and(|e| e == 60));
    //}
}
