use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub struct CircularRingBuffer<T, const N: usize> {
    buffer: [Option<T>; N],
    head: AtomicUsize,
    tail: AtomicUsize,
    overflowed: AtomicBool,
}

impl<T, const N: usize> CircularRingBuffer<T, N> {
    pub const fn new() -> Self {
        Self {
            buffer: [const { None }; N],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            overflowed: AtomicBool::new(false),
        }
    }

    pub fn write(&self, value: T) {
        let mut increased_tail = false;
        match self
            .overflowed
            .compare_exchange(true, false, Ordering::Acquire, Ordering::Relaxed)
        {
            Ok(..) => {
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
                increased_tail = true;
            }
            Err(..) => {}
        };

        let mut head = self.head.load(Ordering::Acquire);
        let mut new_head;
        let mut overflowed;
        loop {
            new_head = (head + 1) % N;
            overflowed = new_head == 0;

            // If the overflow is set and increase_tail is also set it will be invalid state
            // because you cannot overflow in two write
            if !(overflowed && increased_tail) {
                match self.overflowed.compare_exchange(
                    false,
                    overflowed,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                ) {
                    Ok(..) => {}
                    Err(..) => {
                        if overflowed {
                            head = self.head.load(Ordering::Acquire);
                            continue;
                        }
                    }
                }
            }

            match self
                .head
                .compare_exchange(head, new_head, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(head) => {
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
                        return unsafe {
                            (*(self as *const _ as *mut CircularRingBuffer<T, N>)).buffer[tail]
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
