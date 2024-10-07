use core::sync::atomic::{AtomicUsize, Ordering};

pub struct CircularRingBuffer<T, const N: usize> {
    buffer: [Option<T>; N],
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl<T, const N: usize> CircularRingBuffer<T, N> {
    pub const fn new() -> Self {
        Self {
            buffer: [const { None }; N],
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
                    if head == 0 {
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
                    return unsafe {
                        (*(self as *const _ as *mut CircularRingBuffer<T, N>)).buffer[head] =
                            Some(value);
                    };
                }
                Err(updated) => head = updated,
            };
        }
    }

    pub fn read(&self) -> Option<T> {
        let mut tail = self.tail.load(Ordering::Acquire);
        let mut new_tail;
        loop {
            new_tail = if self.buffer[match tail.checked_sub(1) {
                Some(tail) => tail,
                None => return None,
            }]
            .is_some()
            {
                let mut result = (tail + 1) % (N + 1);
                if result == 0 {
                    result = 1;
                }
                result
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
                        return unsafe {
                            (*(self as *const _ as *mut CircularRingBuffer<T, N>)).buffer[match tail
                                .checked_sub(1)
                            {
                                Some(tail) => tail,
                                None => return None,
                            }]
                            .take()
                        };
                    }
                    return None;
                }
                Err(updated) => tail = updated,
            }
        }
    }
}
