pub mod ahci_driver;
pub mod ata_driver;

use core::error::Error;
use core::f64;
use core::fmt::Display;

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

    fn write(&mut self, from_sector: u64, data: &[u8], count: usize) -> Result<(), Self::Error>;

    fn read(&mut self, from_sector: u64, data: &mut [u8], count: usize) -> Result<(), Self::Error>;

    fn lba_end(&self) -> u64;
}

pub fn init(frame_allocator: &mut AreaFrameAllocator) {
    ahci_driver::init(frame_allocator);
}
