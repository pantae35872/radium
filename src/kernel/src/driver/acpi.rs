use core::fmt::Display;

use alloc::fmt;
use common::boot::BootInformation;
use sdp::Sdp;
use spin::{Mutex, Once};

use crate::log;

mod sdp;

static ACPI: Once<Mutex<Acpi>> = Once::new();

pub fn init(boot_info: &BootInformation) {
    log!(Trace, "Initializing acpi");
    let acpi = unsafe { Acpi::new(boot_info.rsdp()) };
    ACPI.call_once(|| acpi.into());
}

struct Acpi {
    #[allow(unused)]
    sdp: Sdp,
}

impl Acpi {
    unsafe fn new(rsdp_addr: u64) -> Self {
        unsafe {
            Self {
                sdp: Sdp::new(rsdp_addr),
            }
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
