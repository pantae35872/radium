use core::fmt::Display;

use alloc::fmt;
use aml::{AmlContext, AmlHandle};
use bootbridge::BootBridge;
use madt::{InterruptControllerStructure, IoApicInterruptSourceOverride, Madt};
use pager::{
    address::{Frame, Page, PhysAddr, VirtAddr},
    EntryFlags, Mapper,
};
use rsdt::Xrsdt;
use sdp::Xrsdp;
use spin::{Mutex, Once};

use crate::{
    log,
    memory::{virt_addr_alloc, MMIOBufferInfo, MemoryContext},
};

mod aml;
mod dsdt;
mod fadt;
pub mod madt;
mod rsdt;
mod sdp;

static ACPI: Once<Mutex<Acpi>> = Once::new();

pub fn init(boot_bridge: &BootBridge, ctx: &mut MemoryContext) {
    log!(Trace, "Initializing acpi");
    let acpi = unsafe { Acpi::new(boot_bridge.rsdp(), ctx) };
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
    unsafe fn new(rsdp_addr: PhysAddr, ctx: &mut MemoryContext) -> Self {
        let xrsdp = unsafe { Xrsdp::new(rsdp_addr, ctx) };
        let xrsdt = unsafe { xrsdp.xrsdt(ctx) };
        Self {
            xrsdp,
            xrsdt,
            aml: AmlContext::new(AcpiHandle),
        }
    }

    pub fn local_apic_mmio(&self, ctx: &mut MemoryContext) -> MMIOBufferInfo {
        let madt = self
            .xrsdt
            .get::<Madt>(ctx)
            .expect("MADT table is required for APIC initialization");
        // SAFETY: we know this is safe because this is from acpi tables
        unsafe { MMIOBufferInfo::new_raw(PhysAddr::new(madt.lapic_base().into()), 1) }
    }

    pub fn io_apics(
        &self,
        ctx: &mut MemoryContext,
        mut callback: impl FnMut(MMIOBufferInfo, usize, &mut MemoryContext),
    ) {
        let madt = self
            .xrsdt
            .get::<Madt>(ctx)
            .expect("MADT table is required for APIC initialization");
        madt.iter()
            .filter_map(|e| match e {
                InterruptControllerStructure::IoApic(io_apic) => Some(io_apic),
                _ => None,
            })
            .for_each(|io_apic| {
                (callback)(
                    unsafe { MMIOBufferInfo::new_raw(io_apic.addr(), 1) },
                    io_apic.gsi_base(),
                    ctx,
                )
            });
    }

    pub fn interrupt_overrides(
        &self,
        ctx: &mut MemoryContext,
        mut callback: impl FnMut(&IoApicInterruptSourceOverride, &mut MemoryContext),
    ) {
        let madt = self
            .xrsdt
            .get::<Madt>(ctx)
            .expect("MADT table is required for APIC initialization");
        madt.iter()
            .filter_map(|e| match e {
                InterruptControllerStructure::IoApicInterruptSourceOverride(io_apic) => {
                    Some(io_apic)
                }
                _ => None,
            })
            .for_each(|e| (callback)(e, ctx));
    }

    /// Call the callback with a list of apic or x2apic id
    pub fn processors(
        &self,
        ctx: &mut MemoryContext,
        mut callback: impl FnMut(usize, &mut MemoryContext),
    ) {
        let madt = self
            .xrsdt
            .get::<Madt>(ctx)
            .expect("MADT table is required for Processors initialization");
        madt.iter()
            .filter_map(|e| match e {
                InterruptControllerStructure::LocalApic(proccesor) => {
                    Some(proccesor.apic_id() as usize)
                }
                InterruptControllerStructure::LocalX2Apic(processor) => Some(processor.apic_id()),
                _ => None,
            })
            .for_each(|e| (callback)(e, ctx));
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
    unsafe fn new(address: u64, ctx: &mut MemoryContext) -> Option<&'static AcpiSdt<T>> {
        log!(Trace, "Accessing acpi table. address: {:#x}", address);
        unsafe {
            ctx.mapper().identity_map_by_size(
                Frame::containing_address(PhysAddr::new(address)),
                size_of::<AcpiSdt<EmptySdt>>(),
                EntryFlags::PRESENT | EntryFlags::NO_CACHE,
            )
        };

        let detect_sdt = unsafe { Self::from_raw(VirtAddr::new(address)) };
        let sdt_signature = detect_sdt.signature;
        let sdt_size = detect_sdt.length.into();
        let _ = detect_sdt;
        unsafe {
            ctx.mapper()
                .unmap_addr(Page::containing_address(VirtAddr::new(address)));
        }
        if sdt_signature != T::signature() {
            return None;
        }
        let virt_sdt = virt_addr_alloc(sdt_size);
        unsafe {
            ctx.mapper().map_to_range_by_size(
                virt_sdt,
                Frame::containing_address(PhysAddr::new(address)),
                sdt_size as usize,
                EntryFlags::NO_CACHE,
            )
        };
        let table =
            unsafe { Self::from_raw(virt_sdt.start_address().align_to(PhysAddr::new(address))) };
        table.validate_checksum();
        return Some(table);
    }

    unsafe fn from_raw(address: VirtAddr) -> &'static AcpiSdt<T> {
        unsafe { &*(address.as_ptr()) }
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
