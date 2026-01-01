use core::slice::{self};

use alloc::boxed::Box;

use crate::initialization_context::{InitializationContext, Stage1};

use super::{AcpiSdt, AcpiSdtData, EmptySdt};

#[repr(C)]
pub struct Rsdt {
    sdts: u32, // 32 bit pointer
}

#[repr(C)]
pub struct Xsdt {
    sdts: u32, // use 32 bit pointer because if use a u64 rust wont allow me to create a unalign
               // slice easilly. as it says in the osdev wiki
}

pub enum Xrsdt {
    RSDT(&'static AcpiSdt<Rsdt>),
    XSDT(&'static AcpiSdt<Xsdt>),
}

impl Xrsdt {
    pub unsafe fn new(address: u64, ctx: &mut InitializationContext<Stage1>) -> Option<Self> {
        unsafe { AcpiSdt::<Rsdt>::new(address, ctx).map(|e| Self::RSDT(e)) }
            .or_else(|| unsafe { AcpiSdt::<Xsdt>::new(address, ctx) }.map(|e| Self::XSDT(e)))
    }

    fn iter(&self) -> Box<dyn Iterator<Item = u64> + '_> {
        match self {
            Self::RSDT(rdst) => Box::new(rdst.iter().map(|&e| e as u64)),
            Self::XSDT(xsdt) => xsdt.iter(),
        }
    }

    pub fn get<T: AcpiSdtData>(&self, ctx: &mut InitializationContext<Stage1>) -> Option<&'static AcpiSdt<T>> {
        self.iter().find_map(|e| unsafe { AcpiSdt::<T>::new(e, ctx) })
    }
}

impl AcpiSdt<Rsdt> {
    fn iter(&self) -> slice::Iter<'_, u32> {
        let length = (self.length - size_of::<AcpiSdt<EmptySdt>>() as u32) / 4;
        let others = unsafe { core::slice::from_raw_parts(&self.data.sdts as *const u32, length as usize) };
        others.iter()
    }
}

impl AcpiSdt<Xsdt> {
    fn iter(&self) -> Box<dyn Iterator<Item = u64> + '_> {
        let length = (self.length - size_of::<AcpiSdt<EmptySdt>>() as u32) / 4;
        let others = unsafe { core::slice::from_raw_parts(&self.data.sdts as *const u32, length as usize) };
        Box::new(others.chunks(2).map(|e| (e[0] as u64) | (e[1] as u64) << 32))
    }
}

impl AcpiSdtData for Rsdt {
    fn signature() -> [u8; 4] {
        *b"RSDT"
    }
}

impl AcpiSdtData for Xsdt {
    fn signature() -> [u8; 4] {
        *b"XSDT"
    }
}
