use alloc::string::String;
use pager::{
    EntryFlags, PAGE_SIZE,
    address::{Page, PhysAddr, Size4K, VirtAddr},
    virt_addr_alloc,
};
use sentinel::log;

use crate::{
    initialization_context::{InitializationContext, Stage1},
    memory::Frame,
};

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
        let page_count = size_of::<Rsdp>().div_ceil(PAGE_SIZE as usize);
        unsafe {
            ctx.mapper().identity_map_auto(Frame::containing_address(rsdp_addr), page_count, EntryFlags::NO_CACHE)
        };
        let check_rsdp = unsafe { Rsdp::new(rsdp_addr.as_u64()) };
        check_rsdp.validate();
        let revision = check_rsdp.revision();
        // After revision checking unmap the sdp.
        unsafe {
            let start_page = Page::<Size4K>::containing_address(VirtAddr::new(rsdp_addr.as_u64()));
            let end_page =
                Page::<Size4K>::containing_address(VirtAddr::new(rsdp_addr.as_u64() + size_of::<Rsdp>() as u64 - 1));
            ctx.mapper().unmap_page_ranges(start_page, end_page);
        };
        // Create sdp based on readed revision
        log!(Trace, "Rsdp address: {:#x}", rsdp_addr);
        log!(Info, "Acpi revision: {}", revision);
        let sdp = match revision {
            AcpiRevisions::Rev1 => {
                let virt_rdsp = virt_addr_alloc::<Size4K>(1);
                let page_count = size_of::<Rsdp>().div_ceil(PAGE_SIZE as usize);
                unsafe {
                    ctx.mapper().map_to_auto(
                        virt_rdsp,
                        Frame::containing_address(rsdp_addr),
                        page_count,
                        EntryFlags::PRESENT,
                    )
                };
                Xrsdp::RSDP(unsafe {
                    Rsdp::new(virt_rdsp.start_address().offset_by_page_misalignment::<Size4K>(rsdp_addr).as_u64())
                })
            }
            AcpiRevisions::Rev2 => {
                let virt_xsdp = virt_addr_alloc::<Size4K>(1);
                unsafe {
                    ctx.mapper().map_to_auto(
                        virt_xsdp,
                        Frame::containing_address(rsdp_addr),
                        size_of::<Xsdp>().div_ceil(PAGE_SIZE as usize),
                        EntryFlags::PRESENT,
                    )
                };
                Xrsdp::XSDP(unsafe {
                    Xsdp::new(virt_xsdp.start_address().offset_by_page_misalignment::<Size4K>(rsdp_addr).as_u64())
                })
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
