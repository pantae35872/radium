use std::iter;

use bitflags::bitflags;
use chrono::{DateTime, Datelike, Local};

use crate::build::iso::{
    boot_catalog::BootCatalog,
    writer::{Endian, IsoFileStr, IsoStrA, IsoStrD, TypeWriter},
};

mod boot_catalog;
pub mod writer;

#[derive(Debug, Default)]
pub struct Iso {
    root: Vec<Directory>,
    uefi_fat: Option<Vec<u8>>,
}

impl Iso {
    pub fn new(root: impl FnOnce(&mut DirectoryWriter), uefi_fat: Option<Vec<u8>>) -> Self {
        let mut new_dir = DirectoryWriter::new();
        root(&mut new_dir);
        Self { root: new_dir.directory, uefi_fat }
    }

    pub fn build(mut self) -> Vec<u8> {
        let mut allocator = SectorAllocator::new();
        // 32 KiB arbitary data
        allocator.alloc(32 * 1024);
        fn write_dir(
            parent: Option<DirectoryEntry>,
            dir: &mut Vec<Directory>,
            allocator: &mut SectorAllocator,
        ) -> (u32, u32) {
            let parent = parent.unwrap_or(DirectoryEntry::parent(0, 0));
            let parent_len = parent.clone().write().len();
            let mut entry = Vec::new();
            let len =
                dir.iter().fold(DirectoryEntry::current(0, 0).write().len() + parent_len, |accum, dir| match dir {
                    Directory::File { name, .. } => DirectoryEntry::file(name.clone(), 0, 0).write().len() + accum,
                    Directory::Directory { name, .. } => DirectoryEntry::dir(name.clone(), 0, 0).write().len() + accum,
                });

            let extent_ptr = allocator.alloc(len);
            let current = DirectoryEntry::current(extent_ptr.loc as u32, len as u32);
            entry.extend(current.clone().write());
            if parent.data_length == 0 && parent.extent == 0 {
                entry.extend(DirectoryEntry::parent(extent_ptr.loc as u32, extent_ptr.size as u32 * 2048).write());
            } else {
                entry.extend(parent.write());
            }

            for dir in dir {
                match dir {
                    Directory::File { name, data, ptr } => {
                        let extent_ptr = allocator.alloc(data.len());
                        *ptr = Some(extent_ptr);
                        allocator.write(&extent_ptr, data);
                        entry.extend(
                            DirectoryEntry::file(name.clone(), extent_ptr.loc as u32, data.len() as u32).write(),
                        );
                    }
                    Directory::Directory { name, child } => {
                        let parent = DirectoryEntry::parent(current.extent, current.data_length);
                        let (dir_extent, dir_len) = write_dir(Some(parent), child, allocator);
                        entry.extend(DirectoryEntry::dir(name.clone(), dir_extent as u32, dir_len as u32).write());
                    }
                }
            }
            allocator.write(&extent_ptr, &entry);
            (extent_ptr.loc as u32, extent_ptr.size as u32 * 2048)
        }

        let descriptor_ptr = allocator.alloc(
            VolumeDescriptor { typ: VolumeDescriptorType::Primary, data: [0; _] }.write().len()
                * if self.uefi_fat.is_some() { 3 } else { 2 },
        );

        let boot_descriptor = self.uefi_fat.map(|fat_img| {
            let fat_ptr = allocator.alloc(fat_img.len());
            allocator.write(&fat_ptr, &fat_img);
            let catalog = BootCatalog { rba: fat_ptr.loc as u32, sector_count: fat_ptr.size as u16 }.build();
            let catalog_ptr = allocator.alloc(catalog.len());
            allocator.write(&catalog_ptr, &catalog);
            VolumeDescriptor::from(BootCatalogDescriptor { catalog_location: catalog_ptr.loc as u32 }).write()
        });

        let (root_extent, root_len) = write_dir(None, &mut self.root, &mut allocator);
        let mut descriptors = Vec::new();

        descriptors.extend(
            VolumeDescriptor::from(PrimaryVolumeDescriptor {
                system_identifier: IsoStrA::new(""),
                volume_identifier: IsoStrD::new("Radium bootable image"),
                volume_space_size: allocator.current as u32,
                volume_set_size: 1,
                volume_sequence_number: 1,
                logical_block_size: 2048,
                path_table_size: 0,
                l_lba_path_table_location: 0,
                l_lba_optional_path_table_location: 0,
                m_lba_path_table_location: 0,
                m_lba_optional_path_table_location: 0,
                root_directory_entry: DirectoryEntry::root(root_extent, root_len),
                volume_set_identifier: IsoStrD::new(""),
                publisher_identifier: IsoStrA::new("radium"),
                data_preparer_identifier: IsoStrA::new("dadium build-tool"),
                application_identifier: IsoStrA::new(""),
                copyright_file_identifier: IsoStrD::new(""),
                abstract_file_identifier: IsoStrD::new(""),
                bibliographic_file_identifier: IsoStrD::new(""),
                volume_creation_date: Local::now(),
                volume_modification: Local::now(),
                volume_expiration_date: Local::now().with_year(Local::now().year() + 5).unwrap(),
                volume_effective_date: Local::now().with_year(Local::now().year() + 5).unwrap(),
                application_used: [0; _],
            })
            .write(),
        );

        if let Some(boot_descriptor) = boot_descriptor {
            descriptors.extend(boot_descriptor);
        }

        descriptors.extend(VolumeDescriptor::from(TerminatorDescriptor).write());

        allocator.write(&descriptor_ptr, &descriptors);

        allocator.data.iter().copied().flatten().collect()
    }
}

#[derive(Debug, Default)]
pub struct DirectoryWriter {
    directory: Vec<Directory>,
}

impl DirectoryWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn file(&mut self, name: IsoFileStr, data: Vec<u8>) -> &mut Self {
        self.directory.push(Directory::File { name, data, ptr: None });
        self
    }

    pub fn dir(&mut self, name: IsoFileStr, dir: impl FnOnce(&mut DirectoryWriter)) -> &mut Self {
        let mut new_dir = DirectoryWriter::new();
        dir(&mut new_dir);
        self.directory.push(Directory::Directory { name, child: new_dir.directory });
        self
    }
}

#[derive(Debug, Clone)]
enum Directory {
    File { name: IsoFileStr, data: Vec<u8>, ptr: Option<SectorAllocatorPtr> },
    Directory { name: IsoFileStr, child: Vec<Directory> },
}

#[derive(Debug, Default)]
struct SectorAllocator {
    current: usize,
    data: Vec<[u8; 2048]>,
}

#[derive(Debug, Clone, Copy)]
struct SectorAllocatorPtr {
    loc: usize,
    size: usize,
}

impl SectorAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write(&mut self, ptr: &SectorAllocatorPtr, mut value: &[u8]) {
        for data in self.data[ptr.loc..][..ptr.size].iter_mut() {
            data[..value.len().min(2048)].copy_from_slice(value[..value.len().min(2048)].try_into().unwrap());
            value = &value[value.len().min(2048)..];
        }
    }

    pub fn alloc(&mut self, size: usize) -> SectorAllocatorPtr {
        let size = if size.is_multiple_of(2048) { size / 2048 } else { size / 2048 + 1 };
        let ptr = SectorAllocatorPtr { loc: self.current, size };
        self.data.extend(iter::repeat_n([0; _], size));
        self.current += size;
        ptr
    }
}

#[derive(Debug, Clone)]
struct DirectoryEntry {
    attribute: u8,
    extent: u32,
    data_length: u32,
    recording_date: DateTime<Local>,
    flags: FileFlags,
    identifier: IsoFileStr,
    str_overwrite: Option<u8>,
}

impl DirectoryEntry {
    /// .. directory entry
    pub fn parent(extent: u32, length: u32) -> Self {
        DirectoryEntry {
            attribute: 0,
            extent,
            data_length: length,
            recording_date: Local::now(),
            flags: FileFlags::DIRECTORY,
            identifier: IsoFileStr::new(""),
            str_overwrite: Some(1),
        }
    }

    /// . current directory
    pub fn current(extent: u32, length: u32) -> Self {
        DirectoryEntry {
            attribute: 0,
            extent,
            data_length: length,
            recording_date: Local::now(),
            flags: FileFlags::DIRECTORY,
            identifier: IsoFileStr::new(""),
            str_overwrite: Some(0),
        }
    }

    pub fn file(name: IsoFileStr, extent: u32, length: u32) -> Self {
        DirectoryEntry {
            attribute: 0,
            extent,
            data_length: length,
            recording_date: Local::now(),
            flags: FileFlags::empty(),
            identifier: name,
            str_overwrite: None,
        }
    }

    pub fn dir(name: IsoFileStr, extent: u32, length: u32) -> Self {
        DirectoryEntry {
            attribute: 0,
            extent,
            data_length: length,
            recording_date: Local::now(),
            flags: FileFlags::DIRECTORY,
            identifier: name,
            str_overwrite: None,
        }
    }

    pub fn root(extent: u32, length: u32) -> [u8; 34] {
        DirectoryEntry {
            attribute: 0,
            extent,
            data_length: length,
            recording_date: Local::now(),
            flags: FileFlags::DIRECTORY,
            identifier: IsoFileStr::new(""),
            str_overwrite: Some(0),
        }
        .write()
        .try_into()
        .unwrap()
    }

    pub fn write(self) -> Vec<u8> {
        let mut writer = TypeWriter::new();
        writer
            .write_u8(0)
            .write_u8(self.attribute)
            .write_u32(self.extent, Endian::Both)
            .write_u32(self.data_length, Endian::Both)
            .write_date_time_dir(self.recording_date)
            .write_u8(self.flags.bits())
            .write_u8(0)
            .write_u8(0)
            .write_u16(1, Endian::Both);
        if let Some(v) = self.str_overwrite {
            writer.write_u8(1).write_u8(v);
        } else {
            writer.write_u8(self.identifier.as_ref().len() as u8).write_str(&self.identifier);
        }

        if !writer.len().is_multiple_of(2) {
            writer.write_u8(0);
        }

        assert!(writer.len().is_multiple_of(2));

        let mut writer = writer.buffer_owned();
        // Back patch
        writer[0] = writer.len() as u8;
        writer
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct FileFlags: u8 {
        /// If set, the existence of this file need not be made known to the user (basically a 'hidden' flag.
        const HIDDEN = 1 << 0;
        /// If set, this record describes a directory (in other words, it is a subdirectory extent).
        const DIRECTORY = 1 << 1;
        /// If set, this file is an "Associated File".
        const ASSOCIATED_FILE = 1 << 2;
        /// If set, the extended attribute record contains information about the format of this file.
        const EXTENDED_ATTRIBUTE = 1 << 3;
        /// If set, owner and group permissions are set in the extended attribute record.
        const OWNER = 1 << 4;
        /// If set, this is not the final directory record for this file (for files spanning several extents, for example files over 4GiB long.
        const LONG_FILE = 1 << 7;
    }
}

#[derive(Debug, Clone)]
struct PathTable {
    directory_name: IsoStrD,
    attribute: u8,
    extent: u32,
    parent_directory: u16,
}

impl PathTable {
    pub fn write(self, endian: Endian) -> Vec<u8> {
        let mut writer = TypeWriter::new();
        assert!(self.directory_name.as_ref().len() < 256, "Directory name too long");
        writer
            .write_u8(self.directory_name.as_ref().len() as u8)
            .write_u8(self.attribute)
            .write_u32(self.extent, endian)
            .write_u16(self.parent_directory, endian)
            .write_str(&self.directory_name);

        if !writer.len().is_multiple_of(2) {
            writer.write_u8(0);
        }

        assert!(writer.len().is_multiple_of(2));

        writer.buffer_owned()
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
struct PrimaryVolumeDescriptor {
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
    pub volume_creation_date: DateTime<Local>,
    pub volume_modification: DateTime<Local>,
    pub volume_expiration_date: DateTime<Local>,
    pub volume_effective_date: DateTime<Local>,
    pub application_used: [u8; 512],
}

impl From<PrimaryVolumeDescriptor> for VolumeDescriptor {
    fn from(value: PrimaryVolumeDescriptor) -> Self {
        let mut writer = TypeWriter::new();
        writer
            .write_u8(0x0) // Unused
            .write_str_padded(&value.system_identifier, 32)
            .write_str_padded(&value.volume_identifier, 32)
            .write_bytes(&[0u8; 8])
            .write_u32(value.volume_space_size, writer::Endian::Both)
            .write_bytes(&[0u8; 32])
            .write_u16(value.volume_set_size, writer::Endian::Both)
            .write_u16(value.volume_sequence_number, writer::Endian::Both)
            .write_u16(value.logical_block_size, writer::Endian::Both)
            .write_u32(value.path_table_size, writer::Endian::Both)
            .write_u32(value.l_lba_path_table_location, writer::Endian::Little)
            .write_u32(value.l_lba_optional_path_table_location, writer::Endian::Little)
            .write_u32(value.m_lba_path_table_location, writer::Endian::Big)
            .write_u32(value.m_lba_optional_path_table_location, writer::Endian::Big)
            .write_bytes(&value.root_directory_entry)
            .write_str_padded(&value.volume_set_identifier, 128)
            .write_str_padded(&value.publisher_identifier, 128)
            .write_str_padded(&value.data_preparer_identifier, 128)
            .write_str_padded(&value.application_identifier, 128)
            .write_str_padded(&value.copyright_file_identifier, 37)
            .write_str_padded(&value.abstract_file_identifier, 37)
            .write_str_padded(&value.bibliographic_file_identifier, 37)
            .write_date_time_ascii(value.volume_creation_date)
            .write_date_time_ascii(value.volume_modification)
            .write_date_time_ascii(value.volume_expiration_date)
            .write_date_time_ascii(value.volume_effective_date)
            .write_u8(1)
            .write_u8(0)
            .write_bytes(&value.application_used)
            .write_bytes(&[0u8; 653]);

        VolumeDescriptor { typ: VolumeDescriptorType::Primary, data: writer.buffer_owned().try_into().unwrap() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BootCatalogDescriptor {
    catalog_location: u32,
}

impl From<BootCatalogDescriptor> for VolumeDescriptor {
    fn from(value: BootCatalogDescriptor) -> Self {
        let mut writer = TypeWriter::new();
        writer.write_str_padded_with(&IsoStrA::new("EL TORITO SPECIFICATION"), 64, 0x0);
        writer.write_u32(value.catalog_location, Endian::Little);
        writer.write_bytes(&[0; 1973]);
        VolumeDescriptor { typ: VolumeDescriptorType::BootRecord, data: writer.buffer_owned().try_into().unwrap() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VolumeDescriptor {
    pub typ: VolumeDescriptorType,
    pub data: [u8; 2041],
}

impl VolumeDescriptor {
    pub fn write(&self) -> Vec<u8> {
        let mut writer = TypeWriter::new();
        writer.write_u8(self.typ as u8).write_str(&IsoStrA::new("CD001")).write_u8(1).write_bytes(&self.data);
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
