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
        let mut head = self.head.load(Ordering::Acquire);
        let mut new_head;
        loop {
            new_head = (head + 1) % N;
            if self.buffer[new_head].is_some() {
                return None;
            }

            match self
                .head
                .compare_exchange(head, new_head, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(head) => {
                    self.buffer[head].write(value);
                    break Some(());
                }
                Err(updated) => head = updated,
            };
        }
    }

    pub fn pop(&self) -> Option<T> {
        let mut tail = self.tail.load(Ordering::Acquire);
        let mut new_tail;
        loop {
            new_tail = if self.buffer[tail].is_some() {
                (tail + 1) % N
            } else {
                tail
            };
            match self.tail.compare_exchange_weak(
                tail,
                new_tail,
                Ordering::Release,
                Ordering::Acquire,
            ) {
                Ok(tail) => {
                    if tail != new_tail {
                        return self.buffer[tail].take();
                    }
                    return None;
                }
                Err(updated) => tail = updated,
            }
        }
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

    fn is_some(&self) -> bool {
        loop {
            match self
                .state
                .compare_exchange(2, 2, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(..) => {
                    return true;
                }
                Err(state) => match state {
                    1 => continue,
                    0 => break false,
                    _ => panic!("Invalid state in slot"),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // TODO: this is only simple single threaded testing, do a multithreaded or multicore testing when we have
    // thread

    #[test_case]
    pub fn read_write() {
        let buffer = SpinMPSC::<_, 5>::new();
        assert!(buffer.push(30).is_some());
        assert!(buffer.push(20).is_some());
        assert!(buffer.pop().is_some_and(|e| e == 30));
        assert!(buffer.pop().is_some_and(|e| e == 20));
        assert!(buffer.pop().is_none());
        assert!(buffer.push(40).is_some());
        assert!(buffer.push(50).is_some());
        assert!(buffer.pop().is_some_and(|e| e == 40));
        assert!(buffer.pop().is_some_and(|e| e == 50));
        assert!(buffer.pop().is_none());
    }

    #[test_case]
    pub fn read_write_err() {
        let buffer = SpinMPSC::<_, 6>::new();
        assert!(buffer.push(30).is_some());
        assert!(buffer.push(20).is_some());
        assert!(buffer.push(40).is_some());
        assert!(buffer.push(50).is_some());
        assert!(buffer.push(60).is_some());
        assert!(buffer.push(70).is_none());
        assert!(buffer.pop().is_some_and(|e| e == 30));
        assert!(buffer.pop().is_some_and(|e| e == 20));
        assert!(buffer.pop().is_some_and(|e| e == 40));
        assert!(buffer.pop().is_some_and(|e| e == 50));
        assert!(buffer.pop().is_some_and(|e| e == 60));
        assert!(buffer.pop().is_none());
        assert!(buffer.pop().is_none());
        assert!(buffer.pop().is_none());
        assert!(buffer.pop().is_none());
        assert!(buffer.pop().is_none());

        assert!(buffer.push(30).is_some());
        assert!(buffer.push(20).is_some());
        assert!(buffer.push(40).is_some());
        assert!(buffer.push(50).is_some());
        assert!(buffer.push(60).is_some());
        assert!(buffer.push(70).is_none());
        assert!(buffer.pop().is_some_and(|e| e == 30));
        assert!(buffer.pop().is_some_and(|e| e == 20));
        assert!(buffer.pop().is_some_and(|e| e == 40));
        assert!(buffer.pop().is_some_and(|e| e == 50));
        assert!(buffer.pop().is_some_and(|e| e == 60));
    }

    #[test_case]
    pub fn interleaved_read_write() {
        let buffer = SpinMPSC::<_, 5>::new();

        assert!(buffer.push(10).is_some());
        assert!(buffer.push(20).is_some());

        assert!(buffer.pop().is_some_and(|e| e == 10));

        assert!(buffer.push(30).is_some());
        assert!(buffer.push(40).is_some());
        assert!(buffer.push(50).is_some());

        assert!(buffer.pop().is_some_and(|e| e == 20));
        assert!(buffer.pop().is_some_and(|e| e == 30));
        assert!(buffer.pop().is_some_and(|e| e == 40));
        assert!(buffer.pop().is_some_and(|e| e == 50));
        assert!(buffer.pop().is_none());
    }

    #[test_case]
    pub fn sequential_read_write() {
        let buffer = SpinMPSC::<_, 4>::new();

        assert!(buffer.push(10).is_some());
        assert!(buffer.pop().is_some_and(|e| e == 10));
        assert!(buffer.push(20).is_some());
        assert!(buffer.pop().is_some_and(|e| e == 20));
        assert!(buffer.push(30).is_some());
        assert!(buffer.pop().is_some_and(|e| e == 30));
        assert!(buffer.push(40).is_some());
        assert!(buffer.pop().is_some_and(|e| e == 40));
        assert!(buffer.push(50).is_some());
        assert!(buffer.pop().is_some_and(|e| e == 50));
        assert!(buffer.push(60).is_some());
        assert!(buffer.pop().is_some_and(|e| e == 60));
    }
}
