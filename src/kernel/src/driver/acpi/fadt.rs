use super::AcpiSdtData;

#[allow(unused)]
pub struct Fadt {
    firmware_control: u32,
    dsdt: u32,
    _reserved: u8,
    prefer_power_management_profile: u8,
}

impl AcpiSdtData for Fadt {
    fn signature() -> [u8; 4] {
        *b"FACP"
    }
}
