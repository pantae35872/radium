pub struct RingBuffer<T, const N: usize> {
    buffer: [Option<T>; N],
    head: usize,
    tail: usize,
}

impl<T, const N: usize> RingBuffer<T, N> {
    pub const fn new() -> Self {
        Self { buffer: [const { None }; N], head: 0, tail: 0 }
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

impl<T, const N: usize> Default for RingBuffer<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn read_write() {
        let mut buffer = RingBuffer::<_, 5>::new();
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

    #[test]
    pub fn read_write_overwrite() {
        let mut buffer = RingBuffer::<_, 5>::new();
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

    #[test]
    pub fn interleaved_read_write() {
        let mut buffer = RingBuffer::<_, 5>::new();

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

    #[test]
    pub fn sequential_read_write() {
        let mut buffer = RingBuffer::<_, 4>::new();

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
