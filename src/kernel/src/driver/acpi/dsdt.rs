use super::{AcpiSdt, AcpiSdtData, EmptySdt};

#[repr(C)]
pub struct Dsdt {
    aml_bytes: u8,
}

impl AcpiSdt<Dsdt> {
    pub fn aml(&self) -> &'static [u8] {
        let length = self.length as usize - size_of::<AcpiSdt<EmptySdt>>();
        unsafe { core::slice::from_raw_parts(&self.data.aml_bytes as *const u8, length) }
    }
}

impl AcpiSdtData for Dsdt {
    fn signature() -> [u8; 4] {
        *b"DSDT"
    }
}
