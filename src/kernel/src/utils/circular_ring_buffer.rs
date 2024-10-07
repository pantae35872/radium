use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// A circular ring buffer but matains it order when overflowed
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
        let mut head = self.head.load(Ordering::Acquire);
        let mut new_head = (head + 1) % N;
        let mut overflowed = new_head == 0;
        while let Err(_) =
            self.head
                .compare_exchange(head, new_head, Ordering::Acquire, Ordering::Relaxed)
        {
            head = self.head.load(Ordering::Acquire);
            new_head = (head + 1) % N;
            overflowed = new_head == 0;
        }

        if self.overflowed.load(Ordering::Acquire) {
            let mut tail = self.tail.load(Ordering::Acquire);
            let mut new_tail = (tail + 1) % N;
            while let Err(_) =
                self.tail
                    .compare_exchange(tail, new_tail, Ordering::Acquire, Ordering::Relaxed)
            {
                tail = self.tail.load(Ordering::Acquire);
                new_tail = (tail + 1) % N;
            }
            // Another rare edge case if the another thread executes in this and increase the value
            // and setting the overflowed we need to decrement it because we already increase it
            // above
            match self.overflowed.compare_exchange(
                true,
                false,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(..) => {}
                Err(..) => {
                    let mut tail = self.tail.load(Ordering::Acquire);
                    let mut new_tail = (tail.wrapping_sub(1).wrapping_add(N)) % N;
                    while let Err(_) = self.tail.compare_exchange(
                        tail,
                        new_tail,
                        Ordering::Acquire,
                        Ordering::Relaxed,
                    ) {
                        tail = self.tail.load(Ordering::Acquire);
                        new_tail = (tail.wrapping_sub(1).wrapping_add(N)) % N;
                    }
                }
            };
        }

        // Rare edge case if other threads some how over flow the buffer in this area of code and not increasing the tail
        // and the state get pass to this function we need to increase the tail now and set the
        // overflowed to be true
        match self.overflowed.compare_exchange(
            false,
            overflowed,
            Ordering::Acquire,
            Ordering::Relaxed,
        ) {
            Ok(..) => {}
            Err(..) => {
                let mut tail = self.tail.load(Ordering::Acquire);
                let mut new_tail = (tail + 1) % N;
                while let Err(_) =
                    self.tail
                        .compare_exchange(tail, new_tail, Ordering::Acquire, Ordering::Relaxed)
                {
                    tail = self.tail.load(Ordering::Acquire);
                    new_tail = (tail + 1) % N;
                }
            }
        };

        unsafe {
            (*(self as *const _ as *mut CircularRingBuffer<T, N>)).buffer[head] = Some(value);
        }
    }

    pub fn read(&self) -> Option<T> {
        let mut tail = self.tail.load(Ordering::Acquire);
        let mut new_tail = (tail + 1) % N;

        let mut value =
            unsafe { (*(self as *const _ as *mut CircularRingBuffer<T, N>)).buffer[tail].take() };

        if value.is_some() {
            while let Err(_) =
                self.tail
                    .compare_exchange(tail, new_tail, Ordering::Acquire, Ordering::Relaxed)
            {
                tail = self.tail.load(Ordering::Acquire);
                new_tail = (tail + 1) % N;
                value = unsafe {
                    (*(self as *const _ as *mut CircularRingBuffer<T, N>)).buffer[tail].take()
                };
            }
        }

        return value;
    }
}
