pub mod ahci_driver;
pub mod ata_driver;

use core::error::Error;
use core::f64;
use core::fmt::Display;
use core::future::Future;

use crate::memory::AreaFrameAllocator;
use crate::utils::floorf64;

pub const MAX_SECTOR: f64 = 63.0;
pub const MAX_HEAD: f64 = 255.0;

#[derive(Debug, Clone, Copy)]
pub struct CHS {
    head: u8,
    sector: u8,
    cylinder: u8,
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

pub fn init(frame_allocator: &mut AreaFrameAllocator) {
    ahci_driver::init(frame_allocator);
}
