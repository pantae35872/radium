use core::fmt::Display;

use alloc::{fmt, vec::Vec};
use aml::{AmlContext, AmlHandle};
use bootbridge::BootBridge;
use fadt::Fadt;
use madt::{InterruptControllerStructure, IoApicInterruptSourceOverride, Madt};
use rsdt::Xrsdt;
use sdp::Xrsdp;
use spin::{Mutex, Once};

use crate::{
    log,
    memory::{memory_controller, paging::EntryFlags, virt_addr_alloc},
};

mod aml;
mod dsdt;
mod fadt;
pub mod madt;
mod rsdt;
mod sdp;

static ACPI: Once<Mutex<Acpi>> = Once::new();

pub fn init(boot_bridge: &BootBridge) {
    log!(Trace, "Initializing acpi");
    let acpi = unsafe { Acpi::new(boot_bridge.rsdp()) };
    ACPI.call_once(|| acpi.into());
}

pub fn acpi() -> &'static Mutex<Acpi> {
    ACPI.get().expect("acpi is not initialized")
}

#[allow(unused)]
pub struct Acpi {
    xrsdp: Xrsdp,
    xrsdt: Xrsdt,
    aml: AmlContext,
}

struct AcpiHandle;

impl AmlHandle for AcpiHandle {
    fn write_debug(&self, value: &str) {
        todo!()
    }
}

impl Acpi {
    unsafe fn new(rsdp_addr: u64) -> Self {
        let xrsdp = unsafe { Xrsdp::new(rsdp_addr) };
        let xrsdt = unsafe { xrsdp.xrsdt() };
        Self {
            xrsdp,
            xrsdt,
            aml: AmlContext::new(AcpiHandle),
        }
    }

    pub fn io_apics(&self, mut callback: impl FnMut(u64, usize)) {
        let madt = self
            .xrsdt
            .get::<Madt>()
            .expect("MADT table is required for APIC initialization");
        madt.iter()
            .filter_map(|e| match e {
                InterruptControllerStructure::IoApic(io_apic) => Some(io_apic),
                _ => None,
            })
            .for_each(|io_apic| (callback)(io_apic.addr(), io_apic.gsi_base()));
    }

    pub fn interrupt_overrides(&self, mut callback: impl FnMut(&IoApicInterruptSourceOverride)) {
        let madt = self
            .xrsdt
            .get::<Madt>()
            .expect("MADT table is required for APIC initialization");
        madt.iter()
            .filter_map(|e| match e {
                InterruptControllerStructure::IoApicInterruptSourceOverride(io_apic) => {
                    Some(io_apic)
                }
                _ => None,
            })
            .for_each(|e| (callback)(e));
    }

    fn aml_init(&mut self) {
        let fadt = self
            .xrsdt
            .get::<Fadt>()
            .expect("No dsdt found in acpi table");
        let dsdt = fadt.dsdt();
        //aml::init(dsdt.aml(), &mut self.aml).unwrap();
    }
}

trait AcpiSdtData {
    fn signature() -> [u8; 4];
}

#[repr(C)]
struct AcpiSdt<T: AcpiSdtData> {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creater_id: u32,
    creator_revision: u32,
    data: T,
}

struct EmptySdt {}

impl AcpiSdtData for EmptySdt {
    fn signature() -> [u8; 4] {
        [0; 4]
    }
}

impl<T: AcpiSdtData> AcpiSdt<T> {
    unsafe fn new(address: u64) -> Option<&'static AcpiSdt<T>> {
        log!(Trace, "Accessing acpi table. address: {:#x}", address);
        memory_controller().lock().ident_map(
            size_of::<AcpiSdt<EmptySdt>>() as u64,
            address,
            EntryFlags::PRESENT | EntryFlags::NO_CACHE,
        );

        let detect_sdt = unsafe { Self::from_raw(address) };
        let sdt_signature = detect_sdt.signature;
        let sdt_size = detect_sdt.length.into();
        let _ = detect_sdt;
        memory_controller()
            .lock()
            .unmap_addr(address, size_of::<AcpiSdt<EmptySdt>>() as u64);
        if sdt_signature != T::signature() {
            return None;
        }
        let virt_sdt = virt_addr_alloc(sdt_size);
        let guard = memory_controller().lock().phy_map(
            sdt_size,
            address,
            virt_sdt,
            EntryFlags::PRESENT | EntryFlags::NO_CACHE,
        );
        let table = unsafe { Self::from_raw(guard.apply(virt_sdt)) };
        table.validate_checksum();
        return Some(table);
    }

    unsafe fn from_raw(address: u64) -> &'static AcpiSdt<T> {
        unsafe { &*(address as *const AcpiSdt<T>) }
    }

    /// Validate this table checksum
    pub fn validate_checksum(&self) {
        let bytes: &[u8] = unsafe {
            core::slice::from_raw_parts(
                self as *const AcpiSdt<T> as *const u8,
                self.length as usize,
            )
        };
        if bytes.iter().map(|e| *e as usize).sum::<usize>() % 256 != 0 {
            panic!("Invalid acpi table");
        }
    }
}

#[derive(Debug)]
enum AcpiRevisions {
    Rev1,
    Rev2,
}

#[derive(Debug)]
#[allow(unused)]
struct UnknownAcpiRevision(u8);

impl TryFrom<u8> for AcpiRevisions {
    type Error = UnknownAcpiRevision;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Rev1),
            2 => Ok(Self::Rev2),
            unknown => Err(UnknownAcpiRevision(unknown)),
        }
    }
}

impl Display for AcpiRevisions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rev1 => write!(f, "Rev.1"),
            Self::Rev2 => write!(f, "Rev.2"),
        }
    }
}
