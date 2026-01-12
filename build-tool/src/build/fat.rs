use std::iter;

use bit_field::BitField;
use bitflags::bitflags;
use chrono::{DateTime, Datelike, Local, NaiveDate, Timelike, Utc};

use crate::build::{Directory, fat::writer::Writer};

mod writer;

/// Count of bytes per sector. This value may take on only the following values: 512, 1024, 2048 or 4096
const BYTES_PER_SECTOR: u16 = 512;
/// Number of sectors per allocation unit. This value must be a power of 2 that is greater than 0.
/// The legal values are 1, 2, 4, 8, 16, 32, 64, and 128
const SECTOR_PER_CLUSTER: u8 = 1;
const BYTES_PER_CLUSTER: usize = BYTES_PER_SECTOR as usize * SECTOR_PER_CLUSTER as usize;

pub fn make(root: &Vec<Directory>) -> Vec<u8> {
    let mut fat = Fat::new();
    write_dir(None, "".to_string(), root, &mut fat);

    let fat_entries = fat.entries.len().max(65526) + 2; // Including 2 reserved
    let fat_size_sector = to_sector_aligned(fat_entries * 4);
    // -2 Excluding the 2 reserved sector since it doens't occupy the data area
    let total_sector = fat_entries - 2 * SECTOR_PER_CLUSTER as usize + fat_size_sector;
    let reserved_count = 0x20;
    let total_sector = reserved_count + total_sector;

    let mut writer = Writer::new();
    let bpb = BPB {
        reserved_sector_count: reserved_count as u16,
        media: 0xF8,
        sec_per_trk: 0,
        num_heads: 0,
        hidden_sector_count: 0,
        total_sector: total_sector as u32,
        drv_number: 0x80,
        ext_flags: 0,
        root_cluster: 2,
        fs_info: 1,
        bk_boot_sec: 6,
        fat_count: 1,
        fat_size: fat_size_sector as u32,
        serial_number: Local::now().timestamp_subsec_nanos(),
    };
    let fsinfo = FSInfo { next_free: fat.entries.len() as u32, free_count: (fat_entries - fat.entries.len()) as u32 };

    bpb.write(&mut writer); // Sector 0
    fsinfo.write(&mut writer); // Sector 1
    writer.padded(BYTES_PER_SECTOR as usize * 4); // padd 3 sector
    bpb.write(&mut writer); // Sector 6
    fsinfo.write(&mut writer); // Sector 7
    writer.padded_min(reserved_count * BYTES_PER_SECTOR as usize);

    for entry in fat.entries {
        writer.write_u32(entry);
    }
    writer.padded_min(reserved_count * BYTES_PER_SECTOR as usize + fat_size_sector * BYTES_PER_SECTOR as usize);

    writer.write_bytes(&fat.data);
    writer.padded_min(total_sector * BYTES_PER_SECTOR as usize);

    writer.buffer_owned()
}

fn write_dir(
    parent: Option<DirectoryStructure>,
    name: String,
    dir: &Vec<Directory>,
    fat: &mut Fat,
) -> DirectoryStructure {
    let volume_label = if parent.is_none() {
        Some(DirectoryStructure::new("RADIUM".to_string(), DirectoryAttribute::VOLUME_ID, 0, 0).write())
    } else {
        None
    };
    let volume_label_size = volume_label.as_ref().map(|e| e.len()).unwrap_or(0);
    let dot_len = DirectoryStructure::new(".".to_string(), DirectoryAttribute::empty(), 0, 0).write().len();
    let dotdot_len = DirectoryStructure::new("..".to_string(), DirectoryAttribute::empty(), 0, 0).write().len();
    let len = dir.iter().fold(dot_len + dotdot_len + volume_label_size, |accum, dir| match dir {
        Directory::Directory { name, .. } | Directory::File { name, .. } => {
            DirectoryStructure::new(name.to_string(), DirectoryAttribute::empty(), 0, 0).write().len() + accum
        }
    });

    let dir_ptr = fat.allocate(len);
    let mut dirs: Vec<u8> = Vec::new();
    // Except for the root directory, each directory must contain the following two entries at the
    // beginning of the directory
    if let Some(parent) = parent {
        // The first directory entry must have a directory name set to “.”
        // This dot entry refers to the current directory. Rules listed above for the DIR_Attr
        // field and DIR_FileSize field must be followed. Since the dot entry refers to the
        // current directory (the one containing the dot entry), the contents of the
        // DIR_FstClusLO and DIR_FstClusHI fields must be the same as that of the current directory.
        // All date and time fields must be set to the same value as that for the containing directory.
        dirs.extend(DirectoryStructure::new(".".to_string(), DirectoryAttribute::DIRECTORY, dir_ptr.entry, 0).write());
        dirs.extend(DirectoryStructure::parent(parent).write());
    }
    if let Some(volume_label) = volume_label {
        dirs.extend(volume_label);
    }
    let current = DirectoryStructure::new(name.to_string(), DirectoryAttribute::DIRECTORY, dir_ptr.entry, 0);
    for dir in dir {
        match dir {
            Directory::File { name, data } => {
                let data_ptr = fat.allocate(data.len());
                fat.write(data_ptr, &data);
                dirs.extend(
                    DirectoryStructure::new(
                        name.to_string(),
                        DirectoryAttribute::empty(),
                        data_ptr.entry,
                        data.len() as u32,
                    )
                    .write(),
                );
            }
            Directory::Directory { name, child } => {
                dirs.extend(write_dir(Some(current.clone()), name.to_string(), child, fat).write());
            }
        }
    }

    fat.write(dir_ptr, &dirs);
    current
}

#[derive(Debug, Default)]
struct Fat {
    entries: Vec<u32>,
    data: Vec<u8>,
}

impl Fat {
    pub fn new() -> Self {
        Self { entries: vec![0x0FFFFFF8, 0x08000000 | 0x04000000], data: Vec::new() }
    }

    pub fn write(&mut self, ptr: FatPtr, mut data: &[u8]) {
        if ptr.entry == 0 || ptr.size == 0 {
            return;
        }
        assert!(data.len() <= ptr.size);
        let index = ptr.entry as usize;
        let mut current_index = index;
        loop {
            let data_index = current_index - 2;
            self.data[data_index * BYTES_PER_CLUSTER..][..data.len().min(BYTES_PER_CLUSTER)]
                .copy_from_slice(&data[..data.len().min(BYTES_PER_CLUSTER)]);
            data = &data[data.len().min(BYTES_PER_CLUSTER)..];
            if self.entries[current_index] == 0xFFFFFFFF {
                break;
            }
            current_index = self.entries[current_index] as usize;
        }
    }

    pub fn allocate(&mut self, size: usize) -> FatPtr {
        if size == 0 {
            return FatPtr { entry: 0, size: 0 };
        }

        let start = self.entries.len() as u32;
        let ptr = FatPtr { entry: start, size };
        let required_entries = if size.is_multiple_of(BYTES_PER_CLUSTER) {
            size / BYTES_PER_CLUSTER
        } else {
            size / BYTES_PER_CLUSTER + 1
        };
        for entry in 0..required_entries {
            if entry == required_entries - 1 {
                self.entries.push(0xFFFFFFFF);
            } else {
                // The push the next entry onto the fat, hence the +1
                self.entries.push(start + entry as u32 + 1);
            }
        }
        self.data.extend(iter::repeat_n(0u8, required_entries * BYTES_PER_CLUSTER));
        ptr
    }
}

#[derive(Debug, Clone, Copy)]
struct FatPtr {
    entry: u32,
    size: usize,
}

#[derive(Debug, Clone)]
struct DirectoryStructure {
    name: String,
    attribute: DirectoryAttribute,
    creation_date: DateTime<Local>,
    last_acccessed: DateTime<Local>,
    modification: DateTime<Local>,
    data_cluster: u32,
    file_size: u32,
}

impl DirectoryStructure {
    fn parent(parent: DirectoryStructure) -> Self {
        // The second directory entry must have the directory name set to “..”
        // This dotdot entry refers to the parent of the current directory. Rules listed above for
        // the DIR_Attr field and DIR_FileSize field must be followed. Since the dotdot
        // entry refers to the parent of the current directory (the one containing the dotdot
        // entry), the contents of the DIR_FstClusLO and DIR_FstClusHI fields must be the
        // same as that of the parent of the current directory. If the parent of the current
        // directory is the root directory (see below), the DIR_FstClusLO and
        // DIR_FstClusHI contents must be set to 0. All date and time fields must be set to
        // the same value as that for the containing directory.
        let mut dotdot = Self { name: "..".to_string(), ..parent };
        if parent.data_cluster == 2 {
            dotdot.data_cluster = 0;
        }
        dotdot
    }

    fn new(name: String, attribute: DirectoryAttribute, data: u32, size: u32) -> Self {
        Self {
            name,
            attribute,
            creation_date: Local::now(),
            last_acccessed: Local::now(),
            modification: Local::now(),
            data_cluster: data,
            file_size: size,
        }
    }

    fn write(&self) -> Vec<u8> {
        let (short_name, long_name) = to_name(&self.name);
        let mut writer = Writer::new();
        if let Some(long_names) = long_name.as_ref() {
            writer.write_bytes(long_names);
        }

        if self.name == "." || self.name == ".." {
            assert!(long_name.is_none());
            writer.write_str_padded(&self.name, 11);
        } else {
            writer.write_bytes(&short_name);
        }
        writer.write_u8(self.attribute.bits());
        writer.write_u8(0);
        writer.write_u8(0);
        writer.write_u16(enc_time(self.creation_date));
        writer.write_u16(enc_date(self.creation_date));
        writer.write_u16(enc_date(self.last_acccessed));
        writer.write_u16(self.data_cluster.get_bits(16..32) as u16);
        writer.write_u16(enc_time(self.modification));
        writer.write_u16(enc_date(self.modification));
        writer.write_u16(self.data_cluster.get_bits(0..16) as u16);
        writer.write_u32(self.file_size);
        assert!(writer.len().is_multiple_of(32));

        writer.buffer_owned()
    }
}

fn to_name(s: &str) -> ([u8; 11], Option<Vec<u8>>) {
    assert!(s.len() <= 255, "File name too long");

    let mut split = s.split(".");
    let (name, ext) = match (split.next(), split.last()) {
        (Some(name), Some(ext)) if name.is_empty() => (ext, None),
        (Some(name), Some(ext)) => (name, Some(ext)),
        _ => (s, None),
    };

    if name.len() > 8 || ext.is_some_and(|e| e.len() > 3) {
        let mut writer = Writer::new();
        write_char_valid(name, &mut writer, 6);
        writer.padded_min_with(6, 0x20);
        writer.write_u8(b'~');
        writer.write_u8(b'1');

        assert_eq!(writer.len(), 8);
        if let Some(ext) = ext {
            write_char_valid(ext, &mut writer, 3);
        }

        writer.padded_min_with(11, 0x20);
        let short_name: [u8; 11] = writer.buffer().try_into().unwrap();

        assert_eq!(writer.buffer().len(), 11);
        let mut cksum = 0u8;
        for c in short_name.iter().copied() {
            cksum = cksum.rotate_right(1).overflowing_add(c).0
        }

        //Names are also NULL terminated and padded with 0xFFFF characters in
        // order to detect corruption of long name fields. A name that fits exactly in a set of long name
        // directory entries (i.e. is an integer multiple of 13) is not NULL terminated and not padded with
        // 0xFFFF.
        let mut chunks = s.encode_utf16().array_chunks::<13>();
        let mut entries = Vec::new();
        let mut i = 0;
        while let Some(chunk) = chunks.next() {
            entries.push(LongNameEntry { ord: (i + 1) as u8, name: chunk, cksum });
            i += 1;
        }

        let remainder = chunks.into_remainder();
        let remainder = remainder.as_slice();
        if !remainder.is_empty() {
            let mut writer = Writer::new();
            for remainder in remainder.iter().copied() {
                writer.write_u16(remainder);
            }
            writer.write_u16(0x00); // NULL terminated
            writer.padded_min_with(13 * 2, 0xFF); // FF padded
            assert_eq!(writer.buffer().len(), 26);
            let name: Vec<u16> =
                writer.buffer().chunks_exact(2).map(|c| u16::from_le_bytes(c.try_into().unwrap())).collect();
            entries.push(LongNameEntry {
                ord: (entries.len() + 1) as u8 | 0x40,
                name: name.try_into().unwrap(),
                cksum,
            });
        } else {
            entries.last_mut().unwrap().ord |= 0x40;
        }

        let mut writer = Writer::new();
        for entry in entries.iter().rev() {
            entry.write(&mut writer);
        }
        return (short_name, Some(writer.buffer_owned()));
    }

    let mut writer = Writer::new();
    write_char_valid(&name, &mut writer, 8);
    writer.padded_min_with(8, 0x20);
    if let Some(ext) = ext {
        write_char_valid(ext, &mut writer, 3);
    }

    writer.padded_min_with(11, 0x20);
    (writer.buffer().try_into().unwrap(), None)
}

fn write_char_valid(s: &str, writer: &mut Writer, max: usize) {
    let valid = "$%'-_@~`!(){}^#&";
    let mut count = 0;
    for c in s.to_ascii_uppercase().chars() {
        count += 1;
        if count > max {
            break;
        }
        if c.is_ascii() {
            match c {
                c if valid.contains(c) || c.is_ascii_uppercase() || c.is_ascii_digit() || c as u8 == 0x20 => {
                    writer.write_u8(c as u8)
                }
                _ => writer.write_u8(b'_'),
            };
        } else {
            writer.write_u8(b'_');
        }
    }
}

struct LongNameEntry {
    ord: u8,
    name: [u16; 13],
    cksum: u8,
}

impl LongNameEntry {
    fn write(&self, writer: &mut Writer) {
        writer.write_u8(self.ord);
        for c in self.name[0..5].iter().copied() {
            writer.write_u16(c);
        }
        writer.write_u8(DirectoryAttribute::LONG_NAME.bits());
        writer.write_u8(0);
        writer.write_u8(self.cksum);
        for c in self.name[5..11].iter().copied() {
            writer.write_u16(c);
        }
        writer.write_u16(0);
        for c in self.name[11..13].iter().copied() {
            writer.write_u16(c);
        }
    }
}

fn enc_time(date: DateTime<Local>) -> u16 {
    let mut time_enc: u16 = 0;
    time_enc.set_bits(0..=4, (date.second() / 2) as u16);
    time_enc.set_bits(5..=10, date.minute() as u16);
    time_enc.set_bits(11..=15, date.hour() as u16);
    time_enc
}

fn enc_date(date: DateTime<Local>) -> u16 {
    let mut date_enc: u16 = 0;
    date_enc.set_bits(0..=4, date.day() as u16);
    date_enc.set_bits(5..=8, date.month() as u16);
    date_enc.set_bits(
        9..=15,
        date.to_utc()
            .years_since(DateTime::from_naive_utc_and_offset(NaiveDate::from_ymd_opt(1900, 1, 1).unwrap().into(), Utc))
            .unwrap() as u16,
    );
    date_enc
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct DirectoryAttribute: u8 {
        const READ_ONLY = 1 << 0;
        const HIDDEN = 1 << 1;
        const SYSTEM = 1 << 2;
        const VOLUME_ID = 1 << 3;
        const DIRECTORY = 1 << 4;
        const ARCHIVE = 1 << 5;
        const LONG_NAME = Self::READ_ONLY.bits() | Self::HIDDEN.bits() | Self::SYSTEM.bits() | Self::VOLUME_ID.bits();
    }
}

#[derive(Debug, Clone)]
struct FSInfo {
    free_count: u32,
    next_free: u32,
}

impl FSInfo {
    pub fn write(&self, writer: &mut Writer) {
        writer.write_u32(0x41615252); // Lead sig
        writer.padded(480);
        writer.write_u32(0x61417272); // Struct sig
        writer.write_u32(self.free_count);
        writer.write_u32(self.next_free);
        writer.padded(12);
        writer.write_u32(0xAA550000);
        assert!(writer.len().is_multiple_of(512));
    }
}

fn to_sector_aligned(bytes: usize) -> usize {
    if bytes.is_multiple_of(BYTES_PER_SECTOR as usize) {
        bytes / BYTES_PER_SECTOR as usize
    } else {
        bytes / BYTES_PER_SECTOR as usize + 1
    }
}

#[derive(Debug, Clone)]
struct BPB {
    /// Number of reserved sectors in the reserved region of the volume starting at the first sector of the volume.
    /// This field is used to align the start of the data area to integral multiples of the cluster size
    /// with respect to the start of the partition/media. This field must not be 0 and can be any non-zero
    /// value. This field should typically be used to align the start of the data area (cluster #2) to the desired
    /// alignment unit, typically cluster size
    reserved_sector_count: u16,
    /// The count of file allocation tables (FATs) on the
    /// volume. A value of 2 is recommended although a
    /// value of 1 is acceptable
    fat_count: u8,
    /// The legal values for this field are 0xF0, 0xF8, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD, 0xFE, and 0xFF.
    /// 0xF8 is the standard value for “fixed” (non-removable) media. For removable media, 0xF0 is
    /// frequently used
    media: u8,
    /// Sectors per track for interrupt 0x13. This field is only relevant for media that have a geometry (volume is broken down into tracks by
    /// multiple heads and cylinders) and are visible on interrupt 0x13.
    sec_per_trk: u16,
    /// Number of heads for interrupt 0x13. This field is relevant as discussed earlier for BPB_SecPerTrk.
    /// This field contains the one based “count of heads”. For example, on a 1.44 MB 3.5-inch floppy drive
    /// this value is 2.
    num_heads: u16,
    /// Count of hidden sectors preceding the partition that contains this FAT volume. This field is generally
    /// only relevant for media visible on interrupt 0x13. This field must always be zero on media that are
    // not partitioned.
    hidden_sector_count: u32,
    /// This field is the 32-bit total count of sectors on the volume.
    /// This count includes the count of all sectors in all four regions of the volume
    total_sector: u32,
    /// This field is the FAT32 32-bit count of sectors occupied by one FAT.
    /// Note that BPB_FATSz16 must be 0 for media formatted FAT32.
    fat_size: u32,
    /// Set as described below:
    /// Bits 0-3 -- Zero-based number of active FAT. Only
    ///             valid if mirroring is disabled.
    /// Bit 7    -- 0 means the FAT is mirrored at runtime into all FATs.
    ///          -- 1 means only one FAT is active; it is the one referenced in bits 0-3.
    /// Bits 4-6 -- Reserved.
    /// Bits 8-15 -- Reserved
    ext_flags: u16,
    /// This is set to the cluster number of the first cluster of the root directory,
    /// This value should be 2 or the first usable (not bad) cluster available thereafter.
    root_cluster: u32,
    /// Sector number of FSINFO structure in the reserved area of the FAT32 volume. Usually 1.
    fs_info: u16,
    /// Set to 0 or 6. If non-zero, indicates the sector number in the reserved area of the volume
    /// of a copy of the bootrecord
    bk_boot_sec: u16,
    /// Interrupt 0x13 drive number. Set value to 0x80 or
    /// 0x00.
    drv_number: u8,
    serial_number: u32,
}

impl BPB {
    pub fn write(&self, writer: &mut Writer) {
        writer.write_u8(0xEB).write_u8(0xFF).write_u8(0x90); // Jmp boot
        writer.write_str_padded("RADIUM", 8); // OEM NAME
        writer.write_u16(BYTES_PER_SECTOR);
        writer.write_u8(SECTOR_PER_CLUSTER);
        writer.write_u16(self.reserved_sector_count);
        writer.write_u8(self.fat_count);
        writer.write_u16(0); // zero for fat 32
        writer.write_u16(0); // zero for fat 32
        writer.write_u8(self.media);
        writer.write_u16(0); // zero for fat 32
        writer.write_u16(self.sec_per_trk);
        writer.write_u16(self.num_heads);
        writer.write_u32(self.hidden_sector_count);
        writer.write_u32(self.total_sector);
        writer.write_u32(self.fat_size);
        writer.write_u16(self.ext_flags);
        writer.write_u16(0); // Revision
        writer.write_u32(self.root_cluster);
        writer.write_u16(self.fs_info);
        writer.write_u16(self.bk_boot_sec);
        writer.padded(12);
        writer.write_u8(self.drv_number);
        writer.padded(1);
        writer.write_u8(0x29);
        writer.write_u32(self.serial_number);
        writer.write_str_padded("RADIUM", 11);
        writer.write_str_padded("FAT32", 8);
        writer.padded(420);
        writer.write_u8(0x55).write_u8(0xAA); // Signature 
        writer.padded(BYTES_PER_SECTOR as usize - 512);
        assert!(writer.len().is_multiple_of(512));
    }
}
