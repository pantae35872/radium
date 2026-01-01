use crate::{
    const_assert,
    initialization_context::{InitializationContext, Stage1},
    inline_if,
};

use super::{AcpiSdt, AcpiSdtData, dsdt::Dsdt};

#[allow(unused)]
#[repr(C, packed)]
pub struct Fadt {
    firmware_control: u32,
    dsdt: u32,
    _reserved: u8,
    prefer_power_management_profile: u8,
    sci_interrupt: u16,
    smi_command_port: u32,
    acpi_enable: u8,
    acpi_disable: u8,
    s4bios_req: u8,
    pstate_control: u8,
    pm1a_event_block: u32,
    pm1b_event_block: u32,
    pm1a_control_block: u32,
    pm1b_control_block: u32,
    pm2_control_block: u32,
    pm_timer_block: u32,
    gpe0_block: u32,
    gpe1_block: u32,
    pm1_event_length: u8,
    pm1_control_length: u8,
    pm2_control_length: u8,
    pm_timer_length: u8,
    gpe0_length: u8,
    gpe1_length: u8,
    gpe1_base: u8,
    cstate_control: u8,
    worst_c2_latency: u16,
    worst_c3_latency: u16,
    flush_size: u16,
    flush_stride: u16,
    duty_offest: u8,
    duty_width: u8,
    day_alarm: u8,
    month_alarm: u8,
    century: u8,
    boot_architecture_flags: u16,
    _reserved2: u8,
    flags: u32,
    pub reset_register: GenericAddressStructure,
    pub reset_value: u8,
    _reserved3: [u8; 3],
    x_firmware_control: u64,
    x_dsdt: u64,
    pub x_pm1a_event_block: GenericAddressStructure,
    x_pm1b_event_block: GenericAddressStructure,
    x_pm1a_control_block: GenericAddressStructure,
    x_pm1b_control_block: GenericAddressStructure,
    x_pm2_control_block: GenericAddressStructure,
    x_pm_timer_block: GenericAddressStructure,
    x_gpe0_block: GenericAddressStructure,
    x_gpe1_block: GenericAddressStructure,
}

const_assert!(size_of::<AcpiSdt<Fadt>>() == 244);

#[derive(Debug)]
#[repr(C, packed)]
pub struct GenericAddressStructure {
    address_space: u8,
    bit_width: u8,
    bit_offset: u8,
    access_size: u8,
    address: u64,
}

const_assert!(size_of::<GenericAddressStructure>() == 12);

impl AcpiSdt<Fadt> {
    pub fn dsdt(&self, ctx: &mut InitializationContext<Stage1>) -> &'static AcpiSdt<Dsdt> {
        unsafe {
            AcpiSdt::<Dsdt>::new(inline_if!(self.data.x_dsdt != 0, self.data.x_dsdt, self.data.dsdt as u64), ctx)
                .expect("Invalid dsdt pointer in fadt")
        }
    }
}

impl AcpiSdtData for Fadt {
    fn signature() -> [u8; 4] {
        *b"FACP"
    }
}
