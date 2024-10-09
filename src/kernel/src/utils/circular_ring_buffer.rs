use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicUsize, Ordering},
};

pub struct CircularRingBuffer<T, const N: usize> {
    buffer: [Slot<T>; N],
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl<T, const N: usize> CircularRingBuffer<T, N> {
    pub const fn new() -> Self {
        Self {
            buffer: [const { Slot::new() }; N],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    pub fn write(&self, value: T) {
        let mut head = self.head.load(Ordering::Acquire);
        let mut new_head;
        loop {
            new_head = (head + 1) % N;

            match self
                .head
                .compare_exchange(head, new_head, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(head) => {
                    if self.buffer[head].is_some() {
                        let mut tail = self.tail.load(Ordering::Acquire);
                        let mut new_tail;
                        loop {
                            new_tail = (tail + 1) % N;
                            match self.tail.compare_exchange(
                                tail,
                                new_tail,
                                Ordering::Acquire,
                                Ordering::Relaxed,
                            ) {
                                Ok(_) => break,
                                Err(updated) => tail = updated,
                            }
                        }
                    }
                    self.buffer[head].write(value);
                    break;
                }
                Err(updated) => head = updated,
            };
        }
    }

    pub fn read(&self) -> Option<T> {
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

unsafe impl<T, const N: usize> Sync for CircularRingBuffer<T, N> {}
unsafe impl<T, const N: usize> Send for CircularRingBuffer<T, N> {}

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
        loop {
            match self
                .state
                .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(..) => {
                    let data = unsafe { &mut *self.data.get() };
                    *data = Some(value);

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
        }
    }

    fn take(&self) -> Option<T> {
        loop {
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
                    0 => break,
                    _ => panic!("Invalid state in slot"),
                },
            }
        }
        return None;
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
                    0 => break,
                    _ => panic!("Invalid state in slot"),
                },
            }
        }
        return false;
    }
}
