use core::{
    fmt::{Arguments, Write},
    mem::offset_of,
    sync::atomic::{AtomicU64, Ordering},
};

use crc::{
    crc64::{self},
    Hasher64,
};

use crate::utils::circular_ring_buffer::{self, lockfree::CircularRingBuffer, singlethreaded};

use super::{CallbackFormatter, LogLevel};

const CHUNK_SIZE: usize = 128;
const DATA_SIZE_PER_CHUNK: usize = CHUNK_SIZE - size_of::<ChunkHeader>();
const HEADER_MAGIC: u64 = u64::from_le_bytes(*b"LogrMagi");

/// One chunk can respresent two possible modes, first is master, second is slave, if the
/// chunk is a master it'll respresent one "log", if the chunk is a slave, it'll just contain the
/// string bytes of the master it's owned to
///
/// "log" is a definition of a one log, it's must only have one log level.
/// Example using log! macro:
/// ```
/// log!(Info, "This is one log ... another 64 charactors");  // This will be respresent as two chunk assuming chunk size is 64 bytes
/// log!(Debug, "This is still one log \n");
/// log!(Trace, "This is one log");
/// ```
/// # Example
/// First chunk:
/// |The header. 32 bytes, is a master (length field != 0), and log level is Info|
/// |32 bytes of the string|
/// Second chunk and ..:
/// |The slaves continue respresenting the string...|
///
///
/// # Warning
/// This when the log is readed, it'll **consume** the log
pub struct StaticLog<const BUFFER_SIZE: usize> {
    buffer: CircularRingBuffer<[u8; CHUNK_SIZE], BUFFER_SIZE>,
    id_count: AtomicU64,
}

/// Contains the header information about the chunk
#[repr(C)]
#[derive(Debug)]
struct ChunkHeader {
    /// Must be equal to HEADER_MAGIC. if this is not equal to HEADER_MAGIC, this chunk must be
    /// ignored
    magic: u64,
    /// if this is zero, this chunk is the slave, if it's is set with something else, then it's the
    /// master, and the field respresent the length in bytes of the string contained in this "log"
    length: u64,
    /// if this chunk is a slave (can be determined using length "read doc of length"), ignore this
    /// field. if this chunk is a master, this will respresent the log level of this "log"
    level: LogLevel,
    /// if this chunk is a master, this will be the chunk master id. if this chunk is a slave, this
    /// will be the master id it's owned to
    id: u64,
    /// Crc sum of the entire chunk excluding the crc sum itself
    /// if the crc is invalid, this chunk must be ignored
    crc64_sum: u64,
}

struct ArgumentCounter {
    counter: usize,
}

impl ArgumentCounter {
    fn new() -> Self {
        Self { counter: 0 }
    }
}

impl Write for ArgumentCounter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.counter += s.len();
        Ok(())
    }
}

struct BufferFiller<F: FnMut(&[u8; DATA_SIZE_PER_CHUNK])> {
    buffer: [u8; DATA_SIZE_PER_CHUNK],
    pos: usize, // current write position in buffer
    process: F,
}

impl<F: FnMut(&[u8; DATA_SIZE_PER_CHUNK])> BufferFiller<F> {
    fn new(process: F) -> Self {
        Self {
            buffer: [0; DATA_SIZE_PER_CHUNK],
            pos: 0,
            process,
        }
    }

    /// It can be called repeatedly with variable-sized input strings.
    fn callback(&mut self, s: &str) {
        let mut bytes = s.as_bytes();

        while !bytes.is_empty() {
            let remaining = DATA_SIZE_PER_CHUNK - self.pos;
            if bytes.len() >= remaining {
                // There are enough bytes to fill the buffer completely.
                // Copy only enough bytes to fill the remainder of the buffer.
                self.buffer[self.pos..].copy_from_slice(&bytes[..remaining]);
                self.pos += remaining;

                // Buffer is full; process it.
                (self.process)(&self.buffer);

                // Reset the buffer
                self.pos = 0;
                self.buffer = [0; DATA_SIZE_PER_CHUNK];

                // Continue processing any leftover bytes.
                bytes = &bytes[remaining..];
            } else {
                // Not enough bytes to fill the buffer entirely.
                let len = bytes.len();
                self.buffer[self.pos..self.pos + len].copy_from_slice(bytes);
                self.pos += len;
                break;
            }
        }
    }

    /// call the process callback if the buffer has something it, that has not been process
    fn flood(&mut self) {
        if self.pos != 0 {
            (self.process)(&self.buffer);
            self.pos = 0;
            self.buffer = [0; DATA_SIZE_PER_CHUNK];
        }
    }
}

impl<const BUFFER_SIZE: usize> StaticLog<BUFFER_SIZE> {
    pub const fn new() -> Self {
        Self {
            buffer: CircularRingBuffer::new(),
            id_count: AtomicU64::new(0),
        }
    }

    /// Push a chunk into the buffer
    fn push_chunk(&self, mut header: ChunkHeader, data: &[u8]) {
        assert_eq!(header.crc64_sum, 0);
        assert!(data.len() <= DATA_SIZE_PER_CHUNK);
        let mut chunk_bytes = [0u8; CHUNK_SIZE];
        let mut digest = crc64::Digest::new(crc64::ECMA);
        chunk_bytes[0..size_of::<ChunkHeader>()].copy_from_slice(&unsafe {
            core::mem::transmute_copy::<ChunkHeader, [u8; size_of::<ChunkHeader>()]>(&header)
        });
        chunk_bytes[size_of::<ChunkHeader>()
            ..size_of::<ChunkHeader>() + DATA_SIZE_PER_CHUNK.min(data.len())]
            .copy_from_slice(data);
        digest.write(&chunk_bytes);
        header.crc64_sum = digest.sum64();
        chunk_bytes[0..size_of::<ChunkHeader>()].copy_from_slice(&unsafe {
            core::mem::transmute_copy::<ChunkHeader, [u8; size_of::<ChunkHeader>()]>(&header)
        });
        self.buffer.write(chunk_bytes);
    }

    fn push_slave(&self, data: &[u8], id: u64) {
        self.push_chunk(
            ChunkHeader {
                magic: HEADER_MAGIC,
                length: 0,
                level: LogLevel::default(),
                id,
                crc64_sum: 0, // Calculate later
            },
            data,
        );
    }

    fn push_master(&self, data: &[u8], length: u64, level: LogLevel, id: u64) {
        self.push_chunk(
            ChunkHeader {
                magic: HEADER_MAGIC,
                length,
                level,
                id,
                crc64_sum: 0, // Calculate later
            },
            data,
        );
    }

    /// Write the master and return it's id
    /// The provided data length must not exceed CHUNK_SIZE - size_of::<ChunkHeader>()
    fn write_master(&self, data: &[u8], length: u64, level: LogLevel) -> u64 {
        let id = self.id_count.fetch_add(1, Ordering::SeqCst);
        self.push_master(data, length, level, id);
        id
    }

    /// Write the slaves into the buffer, the slaves are related to the master id
    /// The provided data length must not exceed CHUNK_SIZE - size_of::<ChunkHeader>()
    fn write_slaves(&self, data: &[u8], master_id: u64) {
        self.push_slave(data, master_id);
    }

    pub fn write_log(&self, log: &Arguments, level: LogLevel) {
        let mut argument_counter = ArgumentCounter::new();
        let _ = argument_counter.write_fmt(*log);
        let mut master_id = None;
        let mut buffer_filler = BufferFiller::new(|buf| match master_id {
            Some(master_id) => {
                self.write_slaves(buf, master_id);
            }
            None => {
                master_id = Some(self.write_master(buf, argument_counter.counter as u64, level));
            }
        });
        let _ = CallbackFormatter::new(|s| buffer_filler.callback(s)).write_fmt(*log);
        buffer_filler.flood();
    }

    fn header_and_data(&self) -> Option<(usize, ChunkHeader, [u8; DATA_SIZE_PER_CHUNK])> {
        let mut lost_bytes = 0;
        let (header, data) = loop {
            lost_bytes += CHUNK_SIZE;

            let mut buffer = self.buffer.read()?;

            let header = unsafe {
                core::mem::transmute::<[u8; size_of::<ChunkHeader>()], ChunkHeader>(
                    TryInto::<[u8; size_of::<ChunkHeader>()]>::try_into(
                        &buffer[0..size_of::<ChunkHeader>()],
                    )
                    .unwrap(),
                )
            };

            if header.magic != HEADER_MAGIC {
                continue;
            }

            buffer[offset_of!(ChunkHeader, crc64_sum)
                ..offset_of!(ChunkHeader, crc64_sum) + size_of::<u64>()]
                .fill(0);

            let mut digest = crc64::Digest::new(crc64::ECMA);
            digest.write(&buffer);
            if header.crc64_sum != digest.sum64() {
                continue;
            }

            lost_bytes -= CHUNK_SIZE;

            let data = TryInto::<[u8; DATA_SIZE_PER_CHUNK]>::try_into(
                &buffer[size_of::<ChunkHeader>()..size_of::<ChunkHeader>() + DATA_SIZE_PER_CHUNK],
            )
            .unwrap();
            break (header, data);
        };

        Some((lost_bytes, header, data))
    }

    fn read_orphan(
        writer: &mut impl Write,
        mut buffer: singlethreaded::CircularRingBuffer<
            (ChunkHeader, [u8; DATA_SIZE_PER_CHUNK]),
            128,
        >,
    ) {
        let mut orphan: singlethreaded::CircularRingBuffer<
            (ChunkHeader, [u8; DATA_SIZE_PER_CHUNK]),
            128,
        > = singlethreaded::CircularRingBuffer::new();
        let mut have_orphan = false;

        let (mut master, data) = loop {
            let (header, data) = match buffer.read() {
                Some(e) => e,
                None => return,
            };

            if header.length == 0 {
                continue;
            }

            break (header, data);
        };

        let _ = writer.write_fmt(format_args!(
            "{}: {}",
            master.level,
            str::from_utf8(&data[..(master.length as usize).min(DATA_SIZE_PER_CHUNK)],).unwrap()
        ));

        master.length -= master.length.min(DATA_SIZE_PER_CHUNK as u64);

        loop {
            if master.length == 0 {
                break;
            }

            let (header, data) = match buffer.read() {
                Some(e) => e,
                None => break,
            };

            if header.length != 0 {
                have_orphan = true;
                orphan.write((header, data));
                continue;
            }

            if header.id != master.id {
                have_orphan = true;
                orphan.write((header, data));
                continue;
            }

            let _ = writer.write_str(
                str::from_utf8(&data[..(master.length as usize).min(DATA_SIZE_PER_CHUNK)]).unwrap(),
            );

            master.length -= master.length.min(DATA_SIZE_PER_CHUNK as u64);
        }

        if have_orphan {
            Self::read_orphan(writer, buffer);
        } else {
            return;
        }
    }

    pub fn read(&self, mut writer: impl Write) -> Option<usize> {
        let mut lost_bytes = 0;
        let mut orphan: singlethreaded::CircularRingBuffer<
            (ChunkHeader, [u8; DATA_SIZE_PER_CHUNK]),
            128,
        > = singlethreaded::CircularRingBuffer::new();
        let (mut lost_bytes, mut master_header, data) = loop {
            lost_bytes += CHUNK_SIZE;

            let (lost, header, data) = self.header_and_data()?;

            lost_bytes += lost;

            if header.length == 0 {
                continue;
            }

            lost_bytes -= CHUNK_SIZE;
            break (lost_bytes + lost, header, data);
        };

        let _ = writer.write_fmt(format_args!(
            "{}: {}",
            master_header.level,
            str::from_utf8(&data[..(master_header.length as usize).min(DATA_SIZE_PER_CHUNK)],)
                .unwrap()
        ));

        master_header.length -= master_header.length.min(DATA_SIZE_PER_CHUNK as u64);

        loop {
            if master_header.length == 0 {
                break;
            }

            let (lost, header, data) = self.header_and_data()?;

            lost_bytes += lost;

            if header.length != 0 {
                orphan.write((header, data));
                continue;
            }

            if header.id != master_header.id {
                orphan.write((header, data));
                continue;
            }

            let _ = writer.write_str(
                str::from_utf8(&data[..(master_header.length as usize).min(DATA_SIZE_PER_CHUNK)])
                    .unwrap(),
            );

            master_header.length -= master_header.length.min(DATA_SIZE_PER_CHUNK as u64);
        }

        Self::read_orphan(&mut writer, orphan);

        Some(lost_bytes)
    }
}

#[cfg(test)]
mod tests {
    use core::fmt::Write;

    use crate::logger::LogLevel;

    use super::{StaticLog, DATA_SIZE_PER_CHUNK};

    struct DummyFormatter<C: Fn(&str, usize)> {
        callback: C,
        counter: usize,
    }

    impl<C> DummyFormatter<C>
    where
        C: Fn(&str, usize),
    {
        pub fn new(callback: C) -> Self {
            Self {
                callback,
                counter: 0,
            }
        }
    }

    impl<C> Write for DummyFormatter<C>
    where
        C: Fn(&str, usize),
    {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            (self.callback)(s, self.counter);
            self.counter += 1;
            Ok(())
        }
    }

    #[test_case]
    fn simple_write() {
        let buffer = StaticLog::<64>::new();
        buffer.write_log(&format_args!("Hello World!!"), LogLevel::Trace);
        buffer.write_log(
            &format_args!("1234567890abcdefghijklmnopqrstuvwxyz3329326964337477803138393016499095029437783814170156256509732480508934402278968101017787386018442744"),
            LogLevel::Trace,
        );
        buffer.write_log(&format_args!("1234567890qwertyuiopasdf"), LogLevel::Trace);

        buffer.read(DummyFormatter::new(|s, c| match (s, c) {
            (s, 0) => assert_eq!(s, "\x1b[94mTRACE\x1b[0m"),
            (s, 1) => assert_eq!(s, ": "),
            (s, 2) => assert_eq!(s, "Hello World!!"),
            _ => unreachable!(),
        }));

        buffer.read(DummyFormatter::new(|s, c| match (s, c) {
            (s, 0) => assert_eq!(s, "\x1b[94mTRACE\x1b[0m"),
            (s, 1) => assert_eq!(s, ": "),
            (s, 2) => assert_eq!(s, "1234567890abcdefghijklmnopqrstuvwxyz3329326964337477803138393016499095029437783814170156"),
            (s, 3) => assert_eq!(s, "256509732480508934402278968101017787386018442744"),
            _ => unreachable!(),
        }));

        buffer.read(DummyFormatter::new(|s, c| match (s, c) {
            (s, 0) => assert_eq!(s, "\x1b[94mTRACE\x1b[0m"),
            (s, 1) => assert_eq!(s, ": "),
            (s, 2) => assert_eq!(s, "1234567890qwertyuiopasdf"),
            _ => unreachable!(),
        }));
    }

    #[test_case]
    fn orphan_write() {
        let buffer = StaticLog::<64>::new();
        let first_id = buffer.write_master(
            &[116; DATA_SIZE_PER_CHUNK],
            DATA_SIZE_PER_CHUNK as u64 * 3,
            LogLevel::Warning,
        );
        buffer.write_slaves(&[116; DATA_SIZE_PER_CHUNK], first_id);
        let second_id = buffer.write_master(
            &[117; DATA_SIZE_PER_CHUNK],
            DATA_SIZE_PER_CHUNK as u64 * 2,
            LogLevel::Info,
        );
        buffer.write_slaves(&[117; DATA_SIZE_PER_CHUNK], second_id);
        buffer.write_slaves(&[116; DATA_SIZE_PER_CHUNK], first_id);

        buffer.read(DummyFormatter::new(|s, c| match (s, c) {
            (s, 0) => assert_eq!(s, "\x1b[93mWARNING\x1b[0m"),
            (s, 1) => assert_eq!(s, ": "),
            (s, 2) => assert_eq!(s, str::from_utf8(&[116; DATA_SIZE_PER_CHUNK]).unwrap()),
            (s, 3) => assert_eq!(s, str::from_utf8(&[116; DATA_SIZE_PER_CHUNK]).unwrap()),
            (s, 4) => assert_eq!(s, str::from_utf8(&[116; DATA_SIZE_PER_CHUNK]).unwrap()),
            (s, 5) => assert_eq!(s, "\x1b[92mINFO\x1b[0m"),
            (s, 6) => assert_eq!(s, ": "),
            (s, 7) => assert_eq!(s, str::from_utf8(&[117; DATA_SIZE_PER_CHUNK]).unwrap()),
            (s, 8) => assert_eq!(s, str::from_utf8(&[117; DATA_SIZE_PER_CHUNK]).unwrap()),
            _ => unreachable!(),
        }));
    }
}
