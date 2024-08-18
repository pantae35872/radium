use core::{error::Error, fmt::Display, mem::size_of};

use crate::driver::storage::{CHSError, Drive, CHS};

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PartitionTableEntry {
    bootable: u8,

    start_chs: CHS,
    partition_id: u8,
    end_chs: CHS,

    start_lba: u32,
    end_lba: u32,
}

#[repr(C)]
struct MasterBootRecord {
    bootloader: [u8; 440],
    signature: u32,
    _unused: u16,

    primary_partition: [PartitionTableEntry; 4],
    magicnumber: u16,
}

impl PartitionTableEntry {
    pub fn new() -> Self {
        Self {
            bootable: 0,
            start_chs: CHS::new(0, 0, 0).expect("Should not failed"),
            partition_id: 0,
            end_chs: CHS::new(0, 0, 0).expect("Should not failed"),
            start_lba: 0,
            end_lba: 0,
        }
    }

    pub fn set_bootable(&mut self, bootable: bool) {
        self.bootable = bootable as u8;
    }

    pub fn set_start_chs(&mut self, chs: CHS) {
        self.start_chs = chs;
    }

    pub fn set_partition_id(&mut self, partition_id: u8) {
        self.partition_id = partition_id;
    }

    pub fn set_end_chs(&mut self, chs: CHS) {
        self.end_chs = chs;
    }

    pub fn set_start_lba(&mut self, start_lba: u32) {
        self.start_lba = start_lba;
    }

    pub fn set_end_lba(&mut self, end_lba: u32) {
        self.end_lba = end_lba;
    }

    pub fn get_bootable(&self) -> bool {
        self.bootable != 0
    }

    pub fn get_start_chs(&self) -> CHS {
        self.start_chs
    }

    pub fn get_end_chs(&self) -> CHS {
        self.end_chs
    }

    pub fn get_id(&self) -> u8 {
        self.partition_id
    }

    pub fn get_start_lba(&self) -> u32 {
        self.start_lba
    }

    pub fn get_end_lba(&self) -> u32 {
        self.end_lba
    }
}

impl MasterBootRecord {
    pub fn new() -> Self {
        Self {
            bootloader: [0; 440],
            signature: 0,
            _unused: 0,
            primary_partition: [PartitionTableEntry::new(); 4],
            magicnumber: 0,
        }
    }
}

pub struct MSDosPartition<'a, T>
where
    T: Drive,
{
    master_boot_record: MasterBootRecord,
    drive: &'a mut T,
}

#[derive(Debug)]
pub enum MSDosPartitionError<T: Error> {
    InvalidMBR,
    SetPartitionChsError(CHSError),
    InvalidPartitionNumber(usize),
    DriveFailed(T),
}

impl<T: Error + Display> Display for MSDosPartitionError<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidMBR => write!(f, "Drive is not mbr"),
            Self::SetPartitionChsError(chs) => {
                write!(f, "Trying to set partition with chs error: {}", chs)
            }
            Self::InvalidPartitionNumber(number) => {
                write!(f, "Invalid partition number {}", number)
            }
            Self::DriveFailed(sata_error) => write!(
                f,
                "Performing mbr partition with drive error: {}",
                sata_error
            ),
        }
    }
}

impl<T: Error + Display> Error for MSDosPartitionError<T> {}

impl<'a, T: Drive> MSDosPartition<'a, T> {
    pub fn new(drive: &'a mut T) -> Self {
        Self {
            master_boot_record: MasterBootRecord::new(),
            drive,
        }
    }

    pub fn load_mbr(&mut self) -> Result<(), MSDosPartitionError<T::Error>> {
        let mbr_bytes: &mut [u8] = unsafe {
            core::slice::from_raw_parts_mut(
                &mut self.master_boot_record as *mut _ as *mut u8,
                size_of::<MasterBootRecord>(),
            )
        };

        self.drive
            .read(0, mbr_bytes, 1)
            .map_err(MSDosPartitionError::DriveFailed)?;

        if self.master_boot_record.magicnumber != 0xAA55 {
            return Err(MSDosPartitionError::InvalidMBR);
        }

        Ok(())
    }

    pub fn save_mbr(&mut self) -> Result<(), MSDosPartitionError<T::Error>> {
        let mbr_bytes: &mut [u8] = unsafe {
            core::slice::from_raw_parts_mut(
                &mut self.master_boot_record as *mut _ as *mut u8,
                size_of::<MasterBootRecord>(),
            )
        };
        self.drive
            .write(0, mbr_bytes, 1)
            .map_err(MSDosPartitionError::DriveFailed)?;
        Ok(())
    }

    pub fn format(&mut self) -> Result<(), MSDosPartitionError<T::Error>> {
        self.master_boot_record = MasterBootRecord::new();
        self.master_boot_record.magicnumber = 0xAA55;
        self.save_mbr()?;
        return Ok(());
    }

    pub fn read_partition(
        &mut self,
        partition_number: usize,
    ) -> Result<PartitionTableEntry, MSDosPartitionError<T::Error>> {
        if partition_number >= 4 {
            return Err(MSDosPartitionError::InvalidPartitionNumber(
                partition_number,
            ));
        }

        Ok(self.master_boot_record.primary_partition[partition_number])
    }

    pub fn set_partition(
        &mut self,
        partition_id: u8,
        partition_number: usize,
        start_lba: u32,
        end_lba: u32,
        bootable: bool,
    ) -> Result<(), MSDosPartitionError<T::Error>> {
        self.load_mbr()?;

        let partition = &mut self.master_boot_record.primary_partition[partition_number];
        partition.set_bootable(bootable);
        partition.set_start_lba(start_lba);
        partition.set_end_lba(end_lba);
        partition.set_partition_id(partition_id);
        let start_chs =
            CHS::from_lba(start_lba).map_err(MSDosPartitionError::SetPartitionChsError)?;
        partition.set_start_chs(start_chs);
        let end_chs = CHS::from_lba((start_lba + end_lba) - 1)
            .map_err(MSDosPartitionError::SetPartitionChsError)?;
        partition.set_end_chs(end_chs);

        self.save_mbr()?;
        return Ok(());
    }
}
