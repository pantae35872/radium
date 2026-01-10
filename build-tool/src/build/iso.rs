use crate::build::iso::writer::{DateTime, IsoStrA, IsoStrD, TypeWriter};

pub mod writer;

#[derive(Debug, Default, Clone)]
pub struct Iso {
    volume_descriptors: Vec<VolumeDescriptor>,
}

impl Iso {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_descriptor<D: Into<VolumeDescriptor>>(&mut self, descriptor: D) {
        self.volume_descriptors.push(descriptor.into());
    }

    pub fn build(self) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(48 * 1024);
        // 32 KiB arbitary data
        buffer.extend_from_slice(&[0u8; 32 * 1024]);
        for descriptor in self.volume_descriptors {
            buffer.extend_from_slice(&descriptor.write());
        }

        buffer
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminatorDescriptor;

impl From<TerminatorDescriptor> for VolumeDescriptor {
    fn from(_value: TerminatorDescriptor) -> Self {
        Self { typ: VolumeDescriptorType::VolumeDescriptorSetTerminator, data: [0u8; _] }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimaryVolumeDescriptor {
    /// The name of the system that can act upon sectors 0x00-0x0F for the volume.
    pub system_identifier: IsoStrA,
    /// Identification of this volume
    pub volume_identifier: IsoStrD,
    /// Number of logical blocks in which the volume is recorded
    pub volume_space_size: u32,
    /// The size of the set in this logical volume (number of disks)
    pub volume_set_size: u16,
    /// The number of this disk in the volume set
    pub volume_sequence_number: u16,
    /// The size in bytes of a logical block
    pub logical_block_size: u16,
    /// The size in bytes of the path table
    pub path_table_size: u32,
    /// little-endian LBA location of the path table
    pub l_lba_path_table_location: u32,
    /// little-endian LBA location of the optional path table, zero for none
    pub l_lba_optional_path_table_location: u32,
    /// little-endian LBA location of the path table
    pub m_lba_path_table_location: u32,
    /// little-endian LBA location of the optional path table, zero for none
    pub m_lba_optional_path_table_location: u32,
    pub root_directory_entry: [u8; 34],
    pub volume_set_identifier: IsoStrD,
    pub publisher_identifier: IsoStrA,
    pub data_preparer_identifier: IsoStrA,
    pub application_identifier: IsoStrA,
    pub copyright_file_identifier: IsoStrD,
    pub abstract_file_identifier: IsoStrD,
    pub bibliographic_file_identifier: IsoStrD,
    pub volume_creation_date: DateTime,
    pub volume_modification: DateTime,
    pub volume_expiration_date: DateTime,
    pub volume_effective_date: DateTime,
    pub application_used: [u8; 512],
}

impl From<PrimaryVolumeDescriptor> for VolumeDescriptor {
    fn from(value: PrimaryVolumeDescriptor) -> Self {
        let mut writer = TypeWriter::new();
        writer.write_u8(0x0); // Unused
        writer.write_str_padded(&value.system_identifier, 32);
        writer.write_str_padded(&value.volume_identifier, 32);
        writer.write_bytes(&[0u8; 8]);
        writer.write_u32(value.volume_space_size, writer::Endian::Both);
        writer.write_bytes(&[0u8; 32]);
        writer.write_u16(value.volume_set_size, writer::Endian::Both);
        writer.write_u16(value.volume_sequence_number, writer::Endian::Both);
        writer.write_u16(value.logical_block_size, writer::Endian::Both);
        writer.write_u32(value.path_table_size, writer::Endian::Both);
        writer.write_u32(value.l_lba_path_table_location, writer::Endian::Little);
        writer.write_u32(value.l_lba_optional_path_table_location, writer::Endian::Little);
        writer.write_u32(value.m_lba_path_table_location, writer::Endian::Big);
        writer.write_u32(value.m_lba_optional_path_table_location, writer::Endian::Big);
        writer.write_bytes(&value.root_directory_entry);
        writer.write_str_padded(&value.volume_set_identifier, 128);
        writer.write_str_padded(&value.publisher_identifier, 128);
        writer.write_str_padded(&value.data_preparer_identifier, 128);
        writer.write_str_padded(&value.application_identifier, 128);
        writer.write_str_padded(&value.copyright_file_identifier, 37);
        writer.write_str_padded(&value.abstract_file_identifier, 37);
        writer.write_str_padded(&value.bibliographic_file_identifier, 37);
        writer.write_date_time(&value.volume_creation_date);
        writer.write_date_time(&value.volume_modification);
        writer.write_date_time(&value.volume_expiration_date);
        writer.write_date_time(&value.volume_effective_date);
        writer.write_u8(1);
        writer.write_u8(0);
        writer.write_bytes(&value.application_used);
        writer.write_bytes(&[0u8; 653]);

        VolumeDescriptor { typ: VolumeDescriptorType::Primary, data: writer.buffer_owned().try_into().unwrap() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootRecord {
    pub boot_system_identifier: IsoStrA,
    pub boot_identifier: IsoStrA,
    pub data: [u8; 1997],
}

impl From<BootRecord> for VolumeDescriptor {
    fn from(value: BootRecord) -> Self {
        let mut writer = TypeWriter::new();
        writer.write_str_padded(&value.boot_system_identifier, 32);
        writer.write_str_padded(&value.boot_identifier, 32);
        writer.write_bytes(&value.data);
        VolumeDescriptor { typ: VolumeDescriptorType::BootRecord, data: writer.buffer_owned().try_into().unwrap() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeDescriptor {
    pub typ: VolumeDescriptorType,
    pub data: [u8; 2041],
}

impl VolumeDescriptor {
    pub fn write(&self) -> Vec<u8> {
        let mut writer = TypeWriter::new();
        writer.write_u8(self.typ as u8);
        writer.write_str(&IsoStrA::new("CD001"));
        writer.write_u8(1);
        writer.write_bytes(&self.data);
        assert_eq!(writer.len(), 2048, "Volume Descriptor must be 2048 bytes in size");

        writer.buffer_owned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum VolumeDescriptorType {
    BootRecord = 0,
    Primary = 1,
    SupplementaryVolumeDescriptor = 2,
    VolumePartitionDescriptor = 3,
    VolumeDescriptorSetTerminator = 255,
}
