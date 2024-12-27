use core::fmt::Display;

use alloc::fmt;
use common::boot::BootInformation;
use fadt::Fadt;
use rsdt::Xrsdt;
use sdp::Xrsdp;
use spin::{Mutex, Once};

use crate::{
    log,
    memory::{memory_controller, paging::EntryFlags, virt_addr_alloc},
};

mod fadt;
mod rsdt;
mod sdp;

static ACPI: Once<Mutex<Acpi>> = Once::new();

pub fn init(boot_info: &BootInformation) {
    log!(Trace, "Initializing acpi");
    let acpi = unsafe { Acpi::new(boot_info.rsdp()) };
    ACPI.call_once(|| acpi.into());
}

#[allow(unused)]
struct Acpi {
    xrsdp: Xrsdp,
    xrsdt: Xrsdt,
}

impl Acpi {
    unsafe fn new(rsdp_addr: u64) -> Self {
        let xrsdp = unsafe { Xrsdp::new(rsdp_addr) };
        let xrsdt = unsafe { xrsdp.xrsdt() };
        let _ = xrsdt.get::<Fadt>().expect("No fadt acpi table found");
        Self { xrsdp, xrsdt }
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
