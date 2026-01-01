use alloc::string::String;
use pager::{
    EntryFlags, Mapper,
    address::{Frame, Page, PhysAddr, VirtAddr},
    virt_addr_alloc,
};
use sentinel::log;

use crate::initialization_context::{InitializationContext, Stage1};

use super::{AcpiRevisions, rsdt::Xrsdt};

#[repr(C, packed)]
pub struct Rsdp {
    signature: [u8; 8],
    checksum: u8,
    oem: [u8; 6],
    revision: u8,
    rsdt_addr: u32,
}

#[repr(C, packed)]
pub struct Xsdp {
    rdsp: Rsdp,
    length: u32,
    xsdt: u64,
    ex_checksum: u8,
    reserved: [u8; 3],
}
#[allow(unused)]
pub enum Xrsdp {
    XSDP(&'static Xsdp),
    RSDP(&'static Rsdp),
}

impl Xrsdp {
    pub unsafe fn new(rsdp_addr: PhysAddr, ctx: &mut InitializationContext<Stage1>) -> Self {
        // Map sdp for revision checking
        unsafe {
            ctx.mapper().identity_map_by_size(
                Frame::containing_address(rsdp_addr),
                size_of::<Rsdp>(),
                EntryFlags::NO_CACHE,
            )
        };
        let check_rsdp = unsafe { Rsdp::new(rsdp_addr.as_u64()) };
        check_rsdp.validate();
        let revision = check_rsdp.revision();
        // After revision checking unmap the sdp.
        unsafe { ctx.mapper().unmap_addr(Page::containing_address(VirtAddr::new(rsdp_addr.as_u64()))) };
        // Create sdp based on readed revision
        log!(Trace, "Rsdp address: {:#x}", rsdp_addr);
        log!(Info, "Acpi revision: {}", revision);
        let sdp = match revision {
            AcpiRevisions::Rev1 => {
                let virt_rdsp = virt_addr_alloc(1);
                unsafe {
                    ctx.mapper().map_to_range_by_size(
                        virt_rdsp,
                        Frame::containing_address(rsdp_addr),
                        size_of::<Rsdp>(),
                        EntryFlags::PRESENT,
                    )
                };
                Xrsdp::RSDP(unsafe { Rsdp::new(virt_rdsp.start_address().align_to(rsdp_addr).as_u64()) })
            }
            AcpiRevisions::Rev2 => {
                let virt_xsdp = virt_addr_alloc(1);
                unsafe {
                    ctx.mapper().map_to_range_by_size(
                        virt_xsdp,
                        Frame::containing_address(rsdp_addr),
                        size_of::<Xsdp>(),
                        EntryFlags::PRESENT,
                    )
                };
                Xrsdp::XSDP(unsafe { Xsdp::new(virt_xsdp.start_address().align_to(rsdp_addr).as_u64()) })
            }
        };
        sdp.validate();
        log!(Trace, "Acpi Oem Id: {}", sdp.oem());

        sdp
    }

    fn rsdp(&self) -> &'static Rsdp {
        match self {
            Self::RSDP(rsdp) => rsdp,
            Self::XSDP(xsdp) => &xsdp.rdsp,
        }
    }

    pub fn oem(&self) -> String {
        self.rsdp().oem.iter().map(|e| *e as char).collect::<String>()
    }

    pub unsafe fn xrsdt(&self, ctx: &mut InitializationContext<Stage1>) -> Xrsdt {
        match self {
            Self::XSDP(xsdp) => unsafe { Xrsdt::new(xsdp.xsdt, ctx).expect("") },
            Self::RSDP(rsdp) => unsafe { Xrsdt::new(rsdp.rsdt_addr as u64, ctx).expect("") },
        }
    }

    fn validate(&self) {
        match self {
            Self::RSDP(rsdp) => rsdp.validate(),
            Self::XSDP(xsdp) => xsdp.validate(),
        }
    }
}

impl Rsdp {
    /// Create a new rsdp from the provided address.
    unsafe fn new(address: u64) -> &'static Self {
        unsafe { &*(address as *const Self) }
    }

    /// Get the revision from this table.
    fn revision(&self) -> AcpiRevisions {
        AcpiRevisions::try_from(self.revision).expect("Unknown acpi revision")
    }

    /// Validate both the signature and checksum
    fn validate(&self) {
        self.validate_signature();
        self.validate_checksum();
    }

    /// Validate the signature ("RSD PTR ") panic if failed.
    fn validate_signature(&self) {
        if &self.signature != b"RSD PTR " {
            panic!("Invalid rdsp signature");
        }
    }

    /// Validate the checksum panic if failed.
    fn validate_checksum(&self) {
        let bytes: &[u8; size_of::<Self>()] = unsafe { core::mem::transmute(self) };
        if bytes.iter().map(|e| *e as usize).sum::<usize>() % 256 != 0 {
            panic!("Invalid rsdp acpi table");
        }
    }
}

impl Xsdp {
    /// Create a new xsdp from the provided address.
    unsafe fn new(address: u64) -> &'static Self {
        unsafe { &*(address as *const Self) }
    }

    /// Validate the checksum and signature
    fn validate(&self) {
        self.rdsp.validate();
        let bytes: &[u8; 36] = unsafe { core::mem::transmute(self) };
        if bytes.iter().map(|e| *e as usize).sum::<usize>() % 256 != 0 {
            panic!("Invalid xsdp acpi table");
        }
    }
}
