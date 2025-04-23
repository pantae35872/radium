use core::arch::asm;

use crate::{
    address::{Frame, PhysAddr},
    Cr3Flags,
};

/// Respresent a [`Cr3`] register in a processor
///
/// [`Cr3`] https://wiki.osdev.org/CPU_Registers_x86#CR3
pub struct Cr3;

impl Cr3 {
    /// Read a [`Frame`] and [`Cr3Flags`] from the cr3 register
    pub fn read() -> (Frame, Cr3Flags) {
        let result: u64;
        // SAFETY: We reading the cr3 is safe we're not setting it
        unsafe {
            asm!("mov {}, cr3", out(reg) result, options(nostack));
        }

        let flags = result & 0xFFF;
        let address = result & !0xFFF; // cut off the first 12 bits (exclusive)

        (
            Frame::containing_address(PhysAddr::new_truncate(address)),
            Cr3Flags::from_bits_truncate(flags),
        )
    }

    pub unsafe fn write(frame: Frame, flags: Cr3Flags) {
        let value = frame.start_address().as_u64() | flags.bits();
        unsafe {
            asm!("mov cr3, {0:r}", in(reg) value, options(nostack));
        }
    }
}

pub mod tlb {
    use crate::address::VirtAddr;

    use super::*;

    #[inline(always)]
    pub fn flush(addr: VirtAddr) {
        unsafe {
            asm!("invlpg [{}]", in(reg) addr.as_u64(), options(nostack, preserves_flags));
        }
    }
}
