use super::{AcpiSdt, AcpiSdtData};

#[derive(Debug)]
#[repr(C, packed)]
pub struct Madt {
    local_apic: u32,
    flags: MultipleApicFlags,
    interrupts: u8,
}

#[derive(Debug, Clone)]
#[repr(C)]
struct InterruptControllerStructureHeader {
    entry_type: u8,
    record_length: u8,
}

#[derive(Debug)]
#[allow(unused)]
pub enum InterruptControllerStructure {
    LocalApic(&'static LocalApic),
    IoApic(&'static IoApic),
    IoApicInterruptSourceOverride(&'static IoApicInterruptSourceOverride),
    IoApicNmi(&'static IoApicNmi),
    LocalApicNmi(&'static LocalApicNmi),
    LocalApicAddressOverride(&'static LocalApicAddressOverride),
    LocalX2Apic(&'static LocalX2Apic),
    Unknown(u8),
}

#[derive(Debug)]
#[repr(C, packed)]
pub struct LocalApic {
    processor_id: u8,
    apic_id: u8,
    flags: LocalApicFlags,
}

#[derive(Debug)]
#[repr(C, packed)]
pub struct IoApic {
    ioapic_id: u8,
    _reserved: u8,
    ioapic_address: u32,
    global_system_interrupt_base: u32,
}

#[derive(Debug)]
#[repr(C, packed)]
pub struct IoApicInterruptSourceOverride {
    bus_source: u8,
    irq_source: u8,
    global_system_interrupt: u32,
    flags: MpsINTIFlags,
}

#[derive(Debug)]
#[repr(C, packed)]
pub struct IoApicNmi {
    nmi_sorce: u8,
    _reserved: u8,
    flags: MpsINTIFlags,
    global_system_interrupt: u32,
}

#[derive(Debug)]
#[repr(C, packed)]
pub struct LocalApicNmi {
    processor_id: u8,
    flags: MpsINTIFlags,
    lint: u8,
}

#[derive(Debug)]
#[repr(C, packed)]
pub struct LocalApicAddressOverride {
    _reserved: [u8; 2],
    address: u64,
}

#[derive(Debug)]
#[repr(C, packed)]
pub struct LocalX2Apic {
    _reserved: [u8; 2],
    local_x2apic_id: u32,
    flags: u32,
    acpi_id: u32,
}

impl AcpiSdt<Madt> {
    pub fn iter(&self) -> MadtInterruptsIter {
        unsafe {
            MadtInterruptsIter::new(
                &self.data.interrupts as *const u8 as u64,
                self.length as u64 - size_of::<AcpiSdt<Madt>>() as u64,
            )
        }
    }

    pub fn apic_base(&self) -> u32 {
        self.data.local_apic
    }
}

pub struct MadtInterruptsIter {
    address: u64,
    end_address: u64,
}

impl MadtInterruptsIter {
    unsafe fn new(address: u64, length: u64) -> Self {
        Self {
            address,
            end_address: address + length - 1,
        }
    }
}

impl Iterator for MadtInterruptsIter {
    type Item = InterruptControllerStructure;

    fn next(&mut self) -> Option<Self::Item> {
        if self.address >= self.end_address {
            return None;
        }

        let header = unsafe { &*(self.address as *const InterruptControllerStructureHeader) };
        let before_addr = self.address;
        self.address += header.record_length as u64;
        Some(unsafe {
            InterruptControllerStructure::from_header_and_pointer(header.clone(), before_addr)
        })
    }
}

impl InterruptControllerStructure {
    unsafe fn from_header_and_pointer(
        header: InterruptControllerStructureHeader,
        header_address: u64,
    ) -> Self {
        match header.entry_type {
            0 => Self::LocalApic(unsafe { Self::calculate_data(header_address) }),
            1 => Self::IoApic(unsafe { Self::calculate_data(header_address) }),
            2 => {
                Self::IoApicInterruptSourceOverride(unsafe { Self::calculate_data(header_address) })
            }
            3 => Self::IoApicNmi(unsafe { Self::calculate_data(header_address) }),
            4 => Self::LocalApicNmi(unsafe { Self::calculate_data(header_address) }),
            5 => Self::LocalApicAddressOverride(unsafe { Self::calculate_data(header_address) }),
            9 => Self::LocalX2Apic(unsafe { Self::calculate_data(header_address) }),
            t => Self::Unknown(t),
        }
    }

    unsafe fn calculate_data<T>(header_address: u64) -> &'static T {
        let header = unsafe { &*(header_address as *const InterruptControllerStructureHeader) };
        assert_eq!(
            size_of::<T>(),
            header.record_length as usize - size_of::<InterruptControllerStructureHeader>()
        );
        unsafe {
            &*((header_address as *const InterruptControllerStructureHeader).offset(1) as *const T)
        }
    }
}

impl AcpiSdtData for Madt {
    fn signature() -> [u8; 4] {
        *b"APIC"
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy)]
    struct LocalApicFlags: u32 {
        const OnlineCapable = 1 << 1;
        const Enabled = 1 << 0;
    }

    #[derive(Debug, Clone, Copy)]
    struct MpsINTIFlags: u16 {
        const PolarityConforms = 0b00;
        const PolarityActiveHigh = 0b01;
        const PolarityActiveLow = 0b11;
        const TriggerModeConforms = 0b00 << 2;
        const TriggerModeEdge = 0b01 << 2;
        const TriggerModeLevel = 0b11 << 2;
    }

    #[derive(Debug, Clone, Copy)]
    struct MultipleApicFlags: u32 {
        const PCATCompatible = 1 << 0;
    }
}