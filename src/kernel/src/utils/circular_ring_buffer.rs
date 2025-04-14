pub struct CircularRingBuffer<T, const N: usize> {
    buffer: [Option<T>; N],
    head: usize,
    tail: usize,
}

impl<T, const N: usize> CircularRingBuffer<T, N> {
    pub const fn new() -> Self {
        Self {
            buffer: [const { None }; N],
            head: 0,
            tail: 0,
        }
    }

    pub fn write(&mut self, value: T) {
        if self.buffer[self.head].is_some() {
            self.tail = (self.tail + 1) % N;
        }
        self.buffer[self.head] = Some(value);
        self.head = (self.head + 1) % N;
    }

    pub fn read(&mut self) -> Option<T> {
        if self.head == self.tail && self.buffer[self.tail].is_none() {
            return None; // Buffer is empty
        }

        let result = self.buffer[self.tail].take();
        if result.is_some() {
            self.tail = (self.tail + 1) % N;
        }

        result
    }
}

impl<T, const N: usize> Default for CircularRingBuffer<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    pub fn read_write() {
        let mut buffer = CircularRingBuffer::<_, 5>::new();
        buffer.write(30);
        buffer.write(20);
        assert_eq!(buffer.read(), Some(30));
        assert_eq!(buffer.read(), Some(20));
        assert!(buffer.read().is_none());
        buffer.write(40);
        buffer.write(50);
        assert_eq!(buffer.read(), Some(40));
        assert_eq!(buffer.read(), Some(50));
        assert!(buffer.read().is_none());
    }

    #[test_case]
    pub fn read_write_overwrite() {
        let mut buffer = CircularRingBuffer::<_, 5>::new();
        buffer.write(30);
        buffer.write(20);
        buffer.write(40);
        buffer.write(50);
        buffer.write(60);
        buffer.write(70);
        assert_eq!(buffer.read(), Some(20));
        assert_eq!(buffer.read(), Some(40));
        assert_eq!(buffer.read(), Some(50));
        assert_eq!(buffer.read(), Some(60));
        assert_eq!(buffer.read(), Some(70));
        assert!(buffer.read().is_none());
    }

    #[test_case]
    pub fn interleaved_read_write() {
        let mut buffer = CircularRingBuffer::<_, 5>::new();

        buffer.write(10);
        buffer.write(20);

        assert_eq!(buffer.read(), Some(10));

        buffer.write(30);
        buffer.write(40);
        buffer.write(50);

        assert_eq!(buffer.read(), Some(20));
        assert_eq!(buffer.read(), Some(30));
        assert_eq!(buffer.read(), Some(40));
        assert_eq!(buffer.read(), Some(50));
        assert!(buffer.read().is_none());
    }

    #[test_case]
    pub fn sequential_read_write() {
        let mut buffer = CircularRingBuffer::<_, 4>::new();

        buffer.write(10);
        assert_eq!(buffer.read(), Some(10));
        buffer.write(20);
        assert_eq!(buffer.read(), Some(20));
        buffer.write(30);
        assert_eq!(buffer.read(), Some(30));
        buffer.write(40);
        assert_eq!(buffer.read(), Some(40));
        buffer.write(50);
        assert_eq!(buffer.read(), Some(50));
        buffer.write(60);
        assert_eq!(buffer.read(), Some(60));
    }
}

pub mod lockfree {
    use core::{
        cell::UnsafeCell,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use x86_64::instructions::interrupts;

    /// Lock free circular ring buffer
    /// this buffer if overflowed will overwrite the oldest data
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

                match self.head.compare_exchange(
                    head,
                    new_head,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                ) {
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

    impl<T, const N: usize> Default for CircularRingBuffer<T, N> {
        fn default() -> Self {
            Self::new()
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
            // this may not be lock free, but it's threaded safe, but not interrupt safe, if interrupts
            // occurs while in writing state it'll be a dead lock, but if it's another core or another
            // thread other core will just wait for this to finish writing, even if it's another thread
            // on the same core it's will *almost* be a dead lock, if the system can gureentee that no
            // thread will be stay on the cpu forever, this will causes no dead lock. but if it's in an interrupts
            // or in an kernel panic. this that "thread" may stay forever causing a dead lock
            interrupts::without_interrupts(|| loop {
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
            interrupts::without_interrupts(|| loop {
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
            // Special cases this does not change the state of the slot, so this dosn't needs interrupt
            // disables
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
            let buffer = CircularRingBuffer::<_, 5>::new();
            buffer.write(30);
            buffer.write(20);
            assert!(buffer.read().is_some_and(|e| e == 30));
            assert!(buffer.read().is_some_and(|e| e == 20));
            assert!(buffer.read().is_none());
            buffer.write(40);
            buffer.write(50);
            assert!(buffer.read().is_some_and(|e| e == 40));
            assert!(buffer.read().is_some_and(|e| e == 50));
            assert!(buffer.read().is_none());
        }

        #[test_case]
        pub fn read_write_overwrite() {
            let buffer = CircularRingBuffer::<_, 5>::new();
            buffer.write(30);
            buffer.write(20);
            buffer.write(40);
            buffer.write(50);
            buffer.write(60);
            buffer.write(70);
            assert!(buffer.read().is_some_and(|e| e == 20));
            assert!(buffer.read().is_some_and(|e| e == 40));
            assert!(buffer.read().is_some_and(|e| e == 50));
            assert!(buffer.read().is_some_and(|e| e == 60));
            assert!(buffer.read().is_some_and(|e| e == 70));
            assert!(buffer.read().is_none());
        }

        #[test_case]
        pub fn interleaved_read_write() {
            let buffer = CircularRingBuffer::<_, 5>::new();

            buffer.write(10);
            buffer.write(20);

            assert!(buffer.read().is_some_and(|e| e == 10));

            buffer.write(30);
            buffer.write(40);
            buffer.write(50);

            assert!(buffer.read().is_some_and(|e| e == 20));
            assert!(buffer.read().is_some_and(|e| e == 30));
            assert!(buffer.read().is_some_and(|e| e == 40));
            assert!(buffer.read().is_some_and(|e| e == 50));
            assert!(buffer.read().is_none());
        }

        #[test_case]
        pub fn sequential_read_write() {
            let buffer = CircularRingBuffer::<_, 4>::new();

            buffer.write(10);
            assert!(buffer.read().is_some_and(|e| e == 10));
            buffer.write(20);
            assert!(buffer.read().is_some_and(|e| e == 20));
            buffer.write(30);
            assert!(buffer.read().is_some_and(|e| e == 30));
            buffer.write(40);
            assert!(buffer.read().is_some_and(|e| e == 40));
            buffer.write(50);
            assert!(buffer.read().is_some_and(|e| e == 50));
            buffer.write(60);
            assert!(buffer.read().is_some_and(|e| e == 60));
        }
    }
}
