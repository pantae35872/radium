use core::{
    fmt::{Arguments, Write},
    num::NonZeroUsize,
    sync::atomic::{AtomicUsize, Ordering},
};

use sink::{lockfree::overwrite::continuous::ContinuousRingBuffer, singlethreaded};
use spin::Mutex;

use super::{CallbackFormatter, LogLevel};

const CHUNK_SIZE: usize = 128;

pub struct StaticLog<const BUFFER_SIZE: usize> {
    buffer: ContinuousRingBuffer<Chunk, BUFFER_SIZE>,
    id_count: AtomicUsize,
    read_buffer: Mutex<singlethreaded::RingBuffer<Chunk, BUFFER_SIZE>>,
}

#[derive(Debug, Clone)]
struct Chunk {
    role: ChunkRole,
    length: usize,
    level: LogLevel,
    id: NonZeroUsize,
    data: [u8; CHUNK_SIZE],
}

#[derive(Debug, Clone)]
enum ChunkRole {
    Start,
    Data,
    End,
}

struct BufferFiller<F: FnMut(&[u8; CHUNK_SIZE], usize)> {
    buffer: [u8; CHUNK_SIZE],
    pos: usize,
    process: F,
}

impl<F: FnMut(&[u8; CHUNK_SIZE], usize)> BufferFiller<F> {
    fn new(process: F) -> Self {
        Self { buffer: [0; CHUNK_SIZE], pos: 0, process }
    }

    fn callback(&mut self, s: &str) {
        // FIXME: We're splitting as bytes but when we piece it togeather some bytes might be on
        // another chunk soo we should fix that
        let mut bytes = s.as_bytes();

        while !bytes.is_empty() {
            let remaining = CHUNK_SIZE - self.pos;

            if bytes.len() >= remaining {
                self.buffer[self.pos..].copy_from_slice(&bytes[..remaining]);
                self.pos += remaining;

                (self.process)(&self.buffer, CHUNK_SIZE);

                self.pos = 0;
                self.buffer = [0; CHUNK_SIZE];

                bytes = &bytes[remaining..];
            } else {
                let len = bytes.len();
                self.buffer[self.pos..self.pos + len].copy_from_slice(bytes);
                self.pos += len;
                break;
            }
        }
    }

    /// call the process callback if the buffer has something it that has not been process
    fn flood(&mut self) {
        if self.pos != 0 {
            (self.process)(&self.buffer, self.pos);
            self.pos = 0;
            self.buffer = [0; CHUNK_SIZE];
        }
    }
}

impl<const BUFFER_SIZE: usize> StaticLog<BUFFER_SIZE> {
    pub const fn new() -> Self {
        Self {
            buffer: ContinuousRingBuffer::new(),
            id_count: AtomicUsize::new(1),
            read_buffer: Mutex::new(singlethreaded::RingBuffer::new()),
        }
    }

    pub fn write_log(&self, log: &Arguments, level: LogLevel) {
        let id = self.id_count.fetch_add(1, Ordering::Relaxed);

        let mut chunk: Option<Chunk> = None;
        let mut first = true;
        let mut buffer_filler = BufferFiller::new(|buf, length| {
            if let Some(mut chunk) = chunk.take() {
                if first {
                    chunk.role = ChunkRole::Start;
                }

                self.buffer.write(chunk);
                first = false;
            }

            chunk =
                Some(Chunk { role: ChunkRole::Data, id: NonZeroUsize::new(id).unwrap(), level, length, data: *buf });
        });

        let _ = CallbackFormatter::new(|s| {
            buffer_filler.callback(s);
        })
        .write_fmt(*log);
        buffer_filler.flood();

        if let Some(mut chunk) = chunk.take() {
            if first {
                chunk.role = ChunkRole::Start;
            } else {
                chunk.role = ChunkRole::End;
            }

            self.buffer.write(chunk);

            if first {
                self.buffer.write(Chunk {
                    role: ChunkRole::End,
                    id: NonZeroUsize::new(id).unwrap(),
                    level,
                    length: 0,
                    data: [0; CHUNK_SIZE],
                });
            }
        }
    }

    pub fn read(&self, mut writer: impl Write) -> usize {
        let mut hold_buffer = self.read_buffer.lock();
        let mut current_id = 0;

        let mut hold_count = 0;
        for chunk in self.buffer.read_cloned() {
            if current_id == 0 && matches!(chunk.role, ChunkRole::Start) {
                current_id = chunk.id.get();
            }

            if chunk.id.get() != current_id {
                hold_buffer.write(chunk);
                hold_count += 1;
                continue;
            }

            match chunk.role {
                ChunkRole::Start => {
                    let _ = writer.write_fmt(format_args!(
                        "[{}] {}",
                        chunk.level,
                        str::from_utf8(&chunk.data[..chunk.length.min(CHUNK_SIZE)]).unwrap_or("?")
                    ));
                }
                ChunkRole::Data => {
                    let _ =
                        writer.write_str(str::from_utf8(&chunk.data[..chunk.length.min(CHUNK_SIZE)]).unwrap_or("?"));
                }
                ChunkRole::End => {
                    let _ =
                        writer.write_str(str::from_utf8(&chunk.data[..chunk.length.min(CHUNK_SIZE)]).unwrap_or("?"));
                    current_id = 0;
                }
            }
        }

        let mut new_hold_count = 0;
        let mut accum_length = 0;
        let mut start_found = false;
        while let Some(chunk) = hold_buffer.read() {
            // complete iteration
            if accum_length >= hold_count {
                // break if there're no more start
                if !start_found {
                    break;
                }

                hold_count = new_hold_count;
                accum_length = 0;
                start_found = false;
            }

            accum_length += 1;
            if current_id == 0 && matches!(chunk.role, ChunkRole::Start) {
                current_id = chunk.id.get();
                start_found = true;
            }

            if chunk.id.get() != current_id {
                hold_buffer.write(chunk);
                new_hold_count += 1;
                continue;
            }

            match chunk.role {
                ChunkRole::Start => {
                    let _ = writer.write_fmt(format_args!(
                        "[{}] {}",
                        chunk.level,
                        str::from_utf8(&chunk.data[..chunk.length.min(CHUNK_SIZE)]).unwrap_or("?")
                    ));
                }
                ChunkRole::Data => {
                    let _ =
                        writer.write_str(str::from_utf8(&chunk.data[..chunk.length.min(CHUNK_SIZE)]).unwrap_or("?"));
                }
                ChunkRole::End => {
                    let _ =
                        writer.write_str(str::from_utf8(&chunk.data[..chunk.length.min(CHUNK_SIZE)]).unwrap_or("?"));
                    current_id = 0;
                }
            }
        }
        return new_hold_count;
    }
}

//struct Connector<const B: usize> {
//    current_id: usize,
//    buffers: [singlethreaded::RingBuffer<Chunk, B>; 2],
//    current_buffer: usize,
//}
//
//impl<const B: usize> Connector<B> {
//    pub const fn new() -> Self {
//        Self { current_id: 0, buffers: array::from_fn(|_| singlethreaded::RingBuffer::new()), current_buffer: 0 }
//    }
//
//    pub fn hold_buffer(&mut self) -> &mut singlethreaded::RingBuffer<Chunk, B> {
//        &mut self.buffers[self.current_buffer.wrapping_add(1) % 2]
//    }
//
//    pub fn current_buffer(&mut self) -> &mut singlethreaded::RingBuffer<Chunk, B> {
//        &mut self.buffers[self.current_buffer % 2]
//    }
//
//    pub fn buffer_of<'a>(&'a mut self, chunk: &Chunk) -> &'a mut singlethreaded::RingBuffer<Chunk, B> {
//        if chunk.id.get() == self.current_id { self.current_buffer() } else { self.hold_buffer() }
//    }
//
//    pub fn swap_buffer(&mut self) {
//        self.current_buffer = self.current_buffer.wrapping_add(1);
//    }
//
//    pub fn connect(&mut self, chunk: Chunk, start: impl FnMut(LogLevel, &[u8])) {
//        match chunk.role {
//            ChunkRole::Start => {
//                if self.current_id == 0 {
//                    self.current_id = chunk.id.get();
//                }
//            }
//            ChunkRole::Data => {}
//            ChunkRole::End => {}
//        }
//
//        self.buffer_of(&chunk).write(chunk.clone());
//
//        if matches!(chunk.role, ChunkRole::End) {
//            while let Some(read) = self.buffer_of(&chunk).read() {
//                process(read)
//            }
//        }
//    }
//}
