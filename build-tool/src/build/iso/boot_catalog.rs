use crate::build::iso::writer::{Endian, IsoStrA, TypeWriter};

#[derive(Debug, Clone)]
pub struct BootCatalog {
    pub sector_count: u16,
    pub rba: u32,
}

impl BootCatalog {
    pub fn build(self) -> Vec<u8> {
        let mut writer = TypeWriter::new();
        writer
            .write_u8(1) // Header Id 01
            .write_u8(0) // Platform Id
            .write_u8(0) // Reserved
            .write_u8(0) // Reserved
            .write_str_padded_with(&IsoStrA::new("RADIUM BOOT"), 24, 0x0);
        let sum = (-(writer
            .buffer()
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes(TryInto::<[u8; 2]>::try_into(chunk).unwrap()))
            .chain([0xAA55])
            .fold(0u16, |ck_sum, value| ck_sum.overflowing_add(value).0) as i16)) as u16;
        writer.write_u16(sum, Endian::Little).write_u8(0x55).write_u8(0xAA);

        assert!(writer.len().is_multiple_of(32));

        writer
            .write_u8(0x88) // Bootable
            .write_u8(0) // No emul
            .write_u16(0, Endian::Little) // Load segment
            .write_u8(0)
            .write_u8(0)
            .write_u16(self.sector_count, Endian::Little)
            .write_u32(self.rba, Endian::Little)
            .write_bytes(&[0; 20]);

        assert!(writer.len().is_multiple_of(32));

        writer
            .write_u8(0x91) // Final header
            .write_u8(0xEF) // UEFI
            .write_u8(1) // Section entry
            .write_bytes(&[0; 29]);

        assert!(writer.len().is_multiple_of(32));

        writer
            .write_u8(0x88) // Bootable
            .write_u8(0) // No emul
            .write_u16(0, Endian::Little) // Load segment
            .write_u8(0)
            .write_u8(0)
            .write_u16(0, Endian::Little)
            .write_u32(self.rba, Endian::Little)
            .write_bytes(&[0; 20]);

        assert!(writer.len().is_multiple_of(32));

        writer.buffer_owned()
    }
}
