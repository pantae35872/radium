use core::arch::asm;

use bitflags::bitflags;

use crate::address::{Frame, PhysAddr, VirtAddr};

/// Respresent a [`Cr3`] register in a processor
///
/// [`Cr3`] https://wiki.osdev.org/CPU_Registers_x86#CR3
pub struct Cr3;

bitflags! {
    /// Contains the [`cr3 flags`] (bits 3-4)
    ///
    /// [`cr3 flags`] https://wiki.osdev.org/CPU_Registers_x86#CR3
    #[derive(PartialEq, Eq, Debug, Clone, Copy)]
    pub struct Cr3Flags: u64 {
        /// Use a writethrough cache policy for the table (otherwise a writeback policy is used).
        const PAGE_LEVEL_WRITETHROUGH = 1 << 3;
        /// Disable caching for the table.
        const PAGE_LEVEL_CACHE_DISABLE = 1 << 4;
    }

    /// Contains the [`cr0 flags`] (bits 3-4)
    ///
    /// [`cr0 flags`] https://wiki.osdev.org/CPU_Registers_x86#CR0
    #[derive(PartialEq, Eq, Debug, Clone, Copy)]
    pub struct Cr0Flags: u64 {
        // TODO: Document this
        const ProtectedModeEnable = 1 << 0;
        const MonitorCoProcessor = 1 << 1;
        const Emulation = 1 << 2;
        const TaskSwitched = 1 << 3;
        const ExtensionType = 1 << 4;
        const NumbericError = 1 << 5;
        const WriteProtect = 1 << 16;
        const AlignmentMask = 1 << 18;
        const NotWriteThrough = 1 << 29;
        const CacheDisable = 1 << 30;
        const Paging = 1 << 31;
    }

    /// Contains the [`efer flags`] (bits 3-4)
    ///
    /// [`efer flags`] http://wiki.osdev.org/CPU_Registers_x86-64#IA32_EFER
    /// [`efer flags`] https://en.wikipedia.org/wiki/Control_register#EFER
    #[derive(PartialEq, Eq, Debug, Clone, Copy)]
    pub struct EferFlags: u64 {
        const SystemCallExtensions = 1 << 0;
        const DPE = 1 << 1; // AMD K6 only
        const SEWBED = 1 << 2; // AMD K6 only
        const GEWBED = 1 << 3; // AMD K6 only
        const L2CacheDisable = 1 << 3; // AMD K6 only
        const LongModeEnable = 1 << 8;
        const LongModeActive = 1 << 10;
        const NoExecuteEnable = 1 << 11;
        const SecureVirtualMachineEnable = 1 << 12;
        const LongModeSegmentLimitEnable = 1 << 13;
        const FastFXSR = 1 << 14;
        const TranslationCacheExtenstion = 1 << 15;
        const MCOMMIT = 1 << 17;
        const INTWB = 1 << 18;
        const UpperAddressIgnoreEnable = 1 << 20;
        const AutomaticIBRSEnable = 1 << 21;
    }


    #[derive(PartialEq, Eq, Debug, Clone, Copy)]
    pub struct RFlagsFlags: u64 {
        const Carry = 1 << 0;
        const ParityFlag = 1 << 2;
        const AuxiliaryCarry = 1 << 4;
        const Zero = 1 << 6;
        const Sign = 1 << 7;
        const Trap = 1 << 8;
        const InterruptEnable = 1 << 9;
        const Direction = 1 << 10;
        const Overflow = 1 << 11;
        const NestedTask = 1 << 14;
        const Resume = 1 << 16;
        const Virtual8086 = 1 << 17;
        const AlignmentCheck = 1 << 18;
        const VirtualInterrupt = 1 << 19;
        const VirtualInterruptPending = 1 << 20;
        const ID = 1 << 21;
    }
}

#[derive(Debug)]
pub struct Msr(u32);
pub struct Cr0;
pub struct Cr2;
pub struct RFlags;
pub struct Efer;
pub struct KernelGsBase;
pub struct GsBase;
pub struct CS;

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct SegmentSelector(pub u16);

impl KernelGsBase {
    const IA32_KERNEL_GS_MSR: Msr = Msr::new(0xc0000102);

    /// Read from the kernel gs base msr as [`VirtAddr`]
    pub fn read() -> VirtAddr {
        unsafe { VirtAddr::new(Self::IA32_KERNEL_GS_MSR.read()) }
    }

    /// Write to the kernel gs base msr from the [`VirtAddr`]
    ///
    /// # Safety
    ///
    /// Caller must ensure that the provided virtual address is pointed to the correctly allocated
    /// memory and mapped
    pub unsafe fn write(addr: VirtAddr) {
        unsafe { Self::IA32_KERNEL_GS_MSR.write(addr.as_u64()) };
    }
}

impl GsBase {
    const IA32_GS_MSR: Msr = Msr::new(0xc0000101);

    /// Read from the kernel gs base msr as [`VirtAddr`]
    pub fn read() -> VirtAddr {
        unsafe { VirtAddr::new(Self::IA32_GS_MSR.read()) }
    }

    /// Write to the kernel gs base msr from the [`VirtAddr`]
    ///
    /// # Safety
    ///
    /// Caller must ensure that the provided virtual address is pointed to the correctly allocated
    /// memory and mapped
    pub unsafe fn write(addr: VirtAddr) {
        unsafe { Self::IA32_GS_MSR.write(addr.as_u64()) };
    }
}

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

    /// Write into cr3 register containing the provided [`Frame`] and [`Cr3Flags`]
    ///
    /// # Safety
    ///
    /// The caller must ensure that changing this does not causes any side effects and the frame is
    /// valid
    pub unsafe fn write(frame: Frame, flags: Cr3Flags) {
        let value = frame.start_address().as_u64() | flags.bits();
        unsafe {
            asm!("mov cr3, {0:r}", in(reg) value, options(nostack));
        }
    }
}

impl Cr2 {
    /// Read from the cr0 flags
    #[inline(always)]
    pub fn read() -> VirtAddr {
        let result: u64;
        // SAFETY: We reading the cr0 is safe we're not setting it
        unsafe {
            asm!("mov {}, cr2", out(reg) result, options(nostack, preserves_flags));
        }

        VirtAddr::new(result)
    }
}

impl Cr0 {
    /// Read from the cr0 into flags
    #[inline(always)]
    pub fn read() -> Cr0Flags {
        let result: u64;
        // SAFETY: We reading the cr0 is safe we're not setting it
        unsafe {
            asm!("mov {}, cr0", out(reg) result, options(nostack, preserves_flags));
        }

        Cr0Flags::from_bits_truncate(result)
    }

    /// Read the cr0 and then perform bitwise or with the provided flags, and write that value back
    ///
    /// # Safety
    ///
    /// the caller must ensure that the provided flags does not cause any unsafe side effects
    #[inline(always)]
    pub unsafe fn write_or(flags: Cr0Flags) {
        let flags = Self::read() | flags;

        unsafe {
            Cr0::write(flags);
        }
    }

    /// Write the flags into the cr0 literally
    ///
    /// # Safety
    ///
    /// the caller must ensure that the provided flags does not cause any unsafe side effects,
    /// or unset the flags that keep the system running
    #[inline(always)]
    pub unsafe fn write(flags: Cr0Flags) {
        unsafe {
            asm!("mov cr0, {}", in(reg) flags.bits(), options(nostack, preserves_flags));
        }
    }
}

impl CS {
    /// Read from the cs segment register
    #[inline(always)]
    pub fn read() -> SegmentSelector {
        let result: u16;
        // SAFETY: We reading the cs is safe we're not setting it
        unsafe {
            asm!("mov {0:x}, cs", out(reg) result, options(nostack, nomem, preserves_flags));
        }

        SegmentSelector(result)
    }

    #[inline(always)]
    pub unsafe fn set(sel: SegmentSelector) {
        unsafe {
            asm!(
                "push {sel}",
                "lea {tmp}, [55f + rip]",
                "push {tmp}",
                "retfq",
                "55:",
                sel = in(reg) u64::from(sel.0),
                tmp = lateout(reg) _,
                options(preserves_flags),
            );
        }
    }
}

impl Msr {
    #[inline(always)]
    pub const fn new(msr: u32) -> Self {
        Self(msr)
    }

    #[inline(always)]
    pub unsafe fn read(&self) -> u64 {
        let (high, low): (u32, u32);
        unsafe {
            asm!(
                "rdmsr",
                in("ecx") self.0,
                out("eax") low, out("edx") high,
                options(nomem, nostack, preserves_flags),
            );
        }
        ((high as u64) << 32) | (low as u64)
    }

    #[inline(always)]
    pub unsafe fn write(&self, value: u64) {
        let low = value as u32;
        let high = (value >> 32) as u32;

        unsafe {
            asm!(
                "wrmsr",
                in("ecx") self.0,
                in("eax") low, in("edx") high,
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}

impl Efer {
    const IA32_EFER_MSR: Msr = Msr::new(0xC0000080);

    /// Read from the [`Efer::IA32_EFER_MSR`] truncate the reserved bits
    pub fn read() -> EferFlags {
        EferFlags::from_bits_truncate(unsafe { Self::IA32_EFER_MSR.read() })
    }

    /// Read the efer msr and then perform bitwise or with the provided flags, and write that value back
    /// if you somehow create an invalid [`EferFlags`] that contains reserved bits, and crash that
    /// on you ¯\_(ツ)_/¯
    ///
    /// # Safety
    ///
    /// the caller must ensure that the provided flags does not cause any unsafe side effects
    #[inline(always)]
    pub unsafe fn write_or(flags: EferFlags) {
        let flags = Self::read() | flags;

        unsafe {
            Self::write(flags);
        }
    }

    /// Write the flags into the msr not preserving any values, if you somehow create an invalid [`EferFlags`]
    /// that contains reserved bits, and crash that on you ¯\_(ツ)_/¯
    ///
    /// # Safety
    ///
    /// the caller must ensure that the provided flags does not cause any unsafe side effects
    #[inline(always)]
    pub unsafe fn write(flags: EferFlags) {
        let msr = Self::IA32_EFER_MSR;
        unsafe { msr.write(flags.bits()) };
    }
}

impl RFlags {
    pub fn read() -> RFlagsFlags {
        let value: u64;
        unsafe {
            asm!("pushfq; pop {:r}", out(reg) value, options(nomem, preserves_flags));
        }

        RFlagsFlags::from_bits_truncate(value)
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed(2))]
pub struct DescriptorTablePointer {
    pub limit: u16,
    pub base: VirtAddr,
}

#[inline(always)]
pub unsafe fn lidt(idt: &DescriptorTablePointer) {
    unsafe {
        asm!("lidt [{}]", in(reg) idt, options(readonly, nostack, preserves_flags));
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
