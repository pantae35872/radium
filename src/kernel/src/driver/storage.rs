pub mod ahci_driver;
pub mod ata_driver;

use core::error::Error;
use core::fmt::Display;
use core::future::Future;
use core::{f64, slice};

use alloc::vec::Vec;
use x86_64::PhysAddr;

use crate::inline_if;
use crate::memory::memory_controller;
use crate::utils::floorf64;

pub const MAX_SECTOR: f64 = 63.0;
pub const MAX_HEAD: f64 = 255.0;

#[derive(Debug, Clone, Copy)]
pub struct CHS {
    head: u8,
    sector: u8,
    cylinder: u8,
}

/// Universal drive command
#[derive(Clone, Copy, Debug)]
enum DriveCommand {
    Read(u64),
    Write(u64),
    Identify,
}

#[derive(Debug)]
struct DmaBuffer {
    start: PhysAddr,
    size: usize, // Size in bytes
    allocated_size: usize,
}

struct DmaRequest {
    command: DriveCommand,
    buffer: Vec<DmaBuffer>,
}

#[derive(Debug)]
pub enum CHSError {
    CylinderError(u16),
    SectorError(u8),
    FailedToParseFromLba(u32),
}

impl Display for CHSError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CylinderError(value) => write!(
                f,
                "Trying to set cylinder with invalid value {}. cylinder cannot be more than 1023",
                value
            ),
            Self::SectorError(value) => write!(
                f,
                "Trying to set sector with invalid value {}. sector cannot be more than 63",
                value
            ),
            Self::FailedToParseFromLba(lba) => {
                write!(f, "Failed to create chs respentations from lba: {}", lba)
            }
        }
    }
}

impl Error for CHSError {}

impl DmaBuffer {
    /// Allocate a buffer for a count and return a left over
    fn new(mut count: usize) -> Option<(Self, usize)> {
        let size = [0x1000, 0x2000, 0x4000]
            .iter()
            .find_map(|e| inline_if!(count <= e >> 9, Some(*e), None))
            .unwrap_or(0x4000);
        let old_count = count;
        count = count.saturating_sub(size >> 9);
        Some((
            Self {
                start: memory_controller().lock().physical_alloc(size)?,
                size: (old_count - count) * 512,
                allocated_size: size,
            },
            count,
        ))
    }

    fn count(&self) -> usize {
        self.size >> 9
    }

    fn copy_into(&self, target: &mut [u8]) {
        assert!(target.len() == self.size);
        memory_controller()
            .lock()
            .ident_map(self.allocated_size as u64, self.start.as_u64());

        let buffer = unsafe { slice::from_raw_parts(self.start.as_u64() as *const u8, self.size) };
        target.copy_from_slice(buffer);

        memory_controller()
            .lock()
            .unmap_addr(self.start.as_u64(), self.allocated_size as u64);
    }

    fn copy_into_self(&self, source: &[u8]) {
        assert!(source.len() == self.size);
        memory_controller()
            .lock()
            .ident_map(self.allocated_size as u64, self.start.as_u64());

        let buffer =
            unsafe { slice::from_raw_parts_mut(self.start.as_u64() as *mut u8, self.size) };
        buffer.copy_from_slice(source);

        memory_controller()
            .lock()
            .unmap_addr(self.start.as_u64(), self.allocated_size as u64);
    }
}

impl Drop for DmaBuffer {
    fn drop(&mut self) {
        let mut memory_alloc = memory_controller().lock();
        memory_alloc.physical_dealloc(self.start, self.allocated_size);
    }
}

impl DriveCommand {
    fn to_ata(&self) -> u8 {
        match self {
            Self::Read(..) => 0x25,
            Self::Write(..) => 0x35,
            Self::Identify => 0xEC,
        }
    }

    fn sector(&self) -> u64 {
        match self {
            Self::Read(value) | Self::Write(value) => *value,
            Self::Identify => 0,
        }
    }

    fn replace_sector(self, new_sector: u64) -> Self {
        match self {
            Self::Read(..) => Self::Read(new_sector),
            Self::Write(..) => Self::Write(new_sector),
            Self::Identify => Self::Identify,
        }
    }

    fn is_write(&self) -> bool {
        matches!(self, Self::Write(..))
    }
}

impl DmaRequest {
    fn new(mut count: usize, command: DriveCommand) -> Option<Self> {
        let mut buffers = Vec::new();

        while count > 0 {
            let (buffer, left_count) = DmaBuffer::new(count)?;

            count = left_count;

            buffers.push(buffer);
        }

        return Some(Self {
            buffer: buffers,
            command,
        });
    }

    fn copy_into(&self, target: &mut [u8]) {
        assert!(target.len() >= self.count() * 512);
        let mut offset = 0;
        for buffer in self.buffer.iter() {
            buffer.copy_into(&mut target[offset..(offset + buffer.size)]);
            offset += buffer.size;
        }
    }

    fn copy_into_self(&mut self, source: &[u8]) {
        assert!(source.len() == self.count() * 512);
        let mut offset = 0;
        for buffer in self.buffer.iter() {
            buffer.copy_into_self(&source[offset..(offset + buffer.size)]);
            offset += buffer.size;
        }
    }

    fn count(&self) -> usize {
        self.buffer.iter().map(|buffer| buffer.count()).sum()
    }
}

impl CHS {
    pub fn new(h: u8, s: u8, c: u16) -> Result<Self, CHSError> {
        let mut chs = Self {
            head: 0,
            sector: 0,
            cylinder: 0,
        };
        chs.set_head(h);
        chs.set_sector(s)?;
        chs.set_cylinder(c)?;
        return Ok(chs);
    }

    pub fn get_cylinder(&self) -> u16 {
        return ((self.sector as u16 & 0b11000000) << 2) | self.cylinder as u16;
    }

    pub fn get_head(&self) -> u8 {
        return self.head;
    }

    pub fn get_sector(&self) -> u8 {
        return self.sector & 0b00111111;
    }

    pub fn set_cylinder(&mut self, cylinder: u16) -> Result<(), CHSError> {
        if cylinder > 1023 {
            return Err(CHSError::CylinderError(cylinder));
        }

        self.sector = (((cylinder >> 2) & 0b11000000) | self.sector as u16) as u8;
        self.cylinder = (cylinder & 0b11111111) as u8;
        return Ok(());
    }

    pub fn set_head(&mut self, head: u8) {
        self.head = head;
    }

    pub fn set_sector(&mut self, sector: u8) -> Result<(), CHSError> {
        if sector > 63 {
            return Err(CHSError::SectorError(sector));
        }

        self.sector = (sector & 0b00111111) | self.sector;
        return Ok(());
    }

    pub fn from_lba(lba: u32) -> Result<Self, CHSError> {
        let cylinder = floorf64((lba as f64) / (MAX_SECTOR * MAX_HEAD as f64));
        let mut work1 = cylinder * (MAX_SECTOR * MAX_HEAD);
        work1 = (lba as f64) - work1;
        let hd = floorf64(work1 / MAX_SECTOR);
        let sct = work1 - hd * MAX_SECTOR + 1.0;

        if cylinder > 1023.0 {
            return Err(CHSError::FailedToParseFromLba(lba));
        }

        return Ok(Self::new(hd as u8, sct as u8, cylinder as u16)?);
    }
}

pub trait Drive {
    type Error: Error;

    /// Write data to the specified sector range on the drive.
    ///
    /// This function writes `count` sectors of data starting at `from_sector`. The underlying device uses a fixed sector size of 512 bytes.
    /// The `data` slice must contain at least `count * 512` bytes. If the slice contains fewer bytes than required, the function will return an error.
    ///
    /// # Parameters
    ///
    /// - `from_sector`: The starting sector on the storage device where the write operation
    /// begins.
    /// - `data`: A slice of bytes containing the data to write. The length of this slice must be
    /// at least `count * 512`
    /// - `count`: The number of sectors to write. The total number of bytes written will be `count * 512`
    ///
    /// # Returns
    ///
    /// - `Ok(())`: If the write operaion succeeds.
    /// - `Err(Self::Error)`: If an error occurs during the write operation, such as an I/O error or if `data` contains fewer bytes than required.
    /// The error type is determined by the implementer of this trait.
    ///
    /// # Example
    ///
    /// ```
    /// let from_sector = 100;
    /// let data = [0u8; 512 * 4];
    /// let count = 4;
    ///
    /// match device.write(from_sector, &data, count) { // Write 4 sector from sector 100
    ///     Ok(()) => println!("Write succeeded."),
    ///     Err(e) => eprintln!("Write failed: {}", e),
    /// }
    ///
    /// ```
    fn write(
        &mut self,
        from_sector: u64,
        data: &[u8],
        count: usize,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Reads data from the specified sector range on the storage device into the provided buffer.
    ///
    /// This function reads `count` sectors of data starting at `from_sector` into the `data` buffer. The underlying device uses a fixed sector size of 512 bytes.
    /// The `data` buffer must be large enough to hold at least `count * 512` bytes. If the buffer contains fewer bytes than required, the function will return an error.
    ///
    /// # Parameters
    ///
    /// - `from_sector`: The starting sector on the storage device where the read operation begins.
    /// - `data`: A mutable slice of bytes where the read data will be stored. The length of this slice must be at least `count * 512` bytes.
    /// - `count`: The number of sectors to read. The total number of bytes read will be `count * 512`.
    ///
    /// # Returns
    ///
    /// - `Ok(())`: If the read operation succeeds.
    /// - `Err(Self::Error)`: If an error occurs during the read operation, such as an I/O error. The specific error type is determined by the implementer of this trait.
    ///
    /// # Example
    ///
    /// ```
    /// let from_sector = 100;
    /// let mut data = [0u8; 512 * 4]; // Buffer for 4 sectors (512 bytes per sector).
    /// let count = 4; // Number of sectors to read.
    ///
    /// match device.read(from_sector, &mut data, count) { // Read 4 sector from sector 100 to `data` buffer
    ///     Ok(()) => println!("Read operation succeeded."),
    ///     Err(e) => eprintln!("Read operation failed: {:?}", e),
    /// }
    fn read(
        &mut self,
        from_sector: u64,
        data: &mut [u8],
        count: usize,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn lba_end(&mut self) -> impl Future<Output = Result<u64, Self::Error>> + Send;
}

pub fn init() {
    ahci_driver::init();
}
