use core::arch::{
    asm,
    x86_64::{_xgetbv, _xsetbv},
};

use bit_field::BitField;
use bitflags::bitflags;
use smart_default::SmartDefault;

use crate::{
    address::{Frame, PhysAddr, VirtAddr},
    PrivilegeLevel,
};

bitflags! {
    /// Contains the [`cr3 flags`] (bits 3-4)
    ///
    /// [`cr3 flags`]: <https://wiki.osdev.org/CPU_Registers_x86#CR3>
    #[derive(PartialEq, Eq, Debug, Clone, Copy)]
    pub struct Cr3Flags: u64 {
        /// Use a writethrough cache policy for the table (otherwise a writeback policy is used).
        const PAGE_LEVEL_WRITETHROUGH = 1 << 3;
        /// Disable caching for the table.
        const PAGE_LEVEL_CACHE_DISABLE = 1 << 4;
    }
}

/// Derived from
///
/// https://www.felixcloutier.com/x86/syscall and
/// https://www.felixcloutier.com/x86/sysret
pub struct SystemCallLStar;

impl SystemCallLStar {
    /// Intel sdm vol 4, page 62
    const IA32_LSTAR_MSR: Msr = Msr::new(0xc0000082);

    /// Read from [Self::IA32_LSTAR_MSR] as [VirtAddr]
    pub fn read() -> VirtAddr {
        unsafe { VirtAddr::new(Self::IA32_LSTAR_MSR.read()) }
    }

    /// Write to the [Self::IA32_LSTAR_MSR] from the [`VirtAddr`]
    ///
    /// # Safety
    ///
    /// Caller must ensure that the provided virtual address is pointed to the executable syscall
    /// function in ring 0
    pub unsafe fn write(addr: VirtAddr) {
        unsafe { Self::IA32_LSTAR_MSR.write(addr.as_u64()) };
    }
}

/// Derived from
///
/// https://www.felixcloutier.com/x86/syscall and
/// https://www.felixcloutier.com/x86/sysret
#[derive(Debug, SmartDefault)]
pub struct SystemCallStar {
    /// the segment selector to be loaded when the syscall instruction is executed
    /// The docs mentioned that the value of CS and SS are derived from
    /// just one selector, and it assumes that the SS register is right above the CS in the gdt
    #[default(SegmentSelector(0))]
    pub syscall_selector: SegmentSelector,
    /// the segment selector to be loaded when the sysret instruction is executed
    /// The docs mentioned that the value of CS and SS are derived from
    /// just one selector, and it just assumes that the SS register is right above the CS in the gdt
    #[default(SegmentSelector(0))]
    pub sysret_selector: SegmentSelector,
}

impl SystemCallStar {
    /// Intel sdm vol 4, page 62
    const IA32_STAR_MSR: Msr = Msr::new(0xc0000081);

    /// Read from the [Self::IA32_STAR_MSR] as [`SystemCallStar`]
    pub fn read() -> Self {
        // The lower half of IA32_STAR_MSR doesn't get mentioned in the documentation
        let msr = unsafe { Self::IA32_STAR_MSR.read() };
        Self {
            // From https://www.felixcloutier.com/x86/syscall
            syscall_selector: SegmentSelector(msr.get_bits(32..48) as u16),
            // From https://www.felixcloutier.com/x86/sysret
            sysret_selector: SegmentSelector(msr.get_bits(48..64) as u16),
        }
    }

    /// Write to the [Self::IA32_STAR_MSR], value are derived from the fields of this struct
    ///
    /// # Safety
    ///
    /// The caller must ensure that the segment following either [Self::syscall_selector] or [Self::sysret_selector]
    /// is a valid segment
    pub unsafe fn write(self) {
        let mut value = 0;
        // The lower half of IA32_STAR_MSR doesn't get mentioned in the documentation
        value.set_bits(32..48, self.syscall_selector.0.into());
        value.set_bits(48..64, self.sysret_selector.0.into());

        unsafe { Self::IA32_STAR_MSR.write(value) };
    }
}

pub struct KernelGsBase;
pub struct GsBase;

impl KernelGsBase {
    /// Intel sdm vol 4, page 62
    const IA32_KERNEL_GS_MSR: Msr = Msr::new(0xc0000102);

    /// Read from the [Self::IA32_KERNEL_GS_MSR] msr as [`VirtAddr`]
    pub fn read() -> VirtAddr {
        unsafe { VirtAddr::new(Self::IA32_KERNEL_GS_MSR.read()) }
    }

    /// Write to the [Self::IA32_KERNEL_GS_MSR] from the [`VirtAddr`]
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
    /// Intel sdm vol 4, page 62
    const IA32_GS_MSR: Msr = Msr::new(0xc0000101);

    /// Read from the [Self::IA32_GS_MSR] as [`VirtAddr`]
    pub fn read() -> VirtAddr {
        unsafe { VirtAddr::new(Self::IA32_GS_MSR.read()) }
    }

    /// Write to the [Self::IA32_GS_MSR] from the [`VirtAddr`]
    ///
    /// # Safety
    ///
    /// Caller must ensure that the provided virtual address is pointed to the correctly allocated
    /// memory and mapped
    pub unsafe fn write(addr: VirtAddr) {
        unsafe { Self::IA32_GS_MSR.write(addr.as_u64()) };
    }

    /// Swap the [GsBase] with [KernelGsBase], using swapgs insruction.
    ///
    /// # Safety
    ///
    /// The caller must ensure that any use of the gs base after this called, act correspondingly
    pub unsafe fn swap() {
        unsafe {
            asm!("swapgs", options(nostack, preserves_flags));
        }
    }
}

/// Respresent a [`Cr3`] register in a processor
///
/// [`Cr3`]: <https://wiki.osdev.org/CPU_Registers_x86#CR3>
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

    /// Reload the cr3 invalidating all tlb
    pub fn reload() {
        let (frame, flags) = Cr3::read();
        unsafe { Cr3::write(frame, flags) }
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

/// Contains a value called Page Fault Linear Address (PFLA). When a page fault occurs,
/// the address the program attempted to access is stored in the [`CR2`] register.
///
/// [`CR2`]: <https://wiki.osdev.org/CPU_Registers_x86#CR3>
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Cr2(VirtAddr);

impl Cr2 {
    /// Read from the cr2 flags
    #[inline]
    pub fn read() -> Self {
        let result: u64;
        // SAFETY: We reading the cr2 is safe we're not setting it
        unsafe {
            asm!("mov {}, cr2", out(reg) result, options(nostack, preserves_flags));
        }

        Self(VirtAddr::new(result))
    }

    #[inline(always)]
    pub fn addr(&self) -> VirtAddr {
        self.0
    }
}

bitflags! {
   /// Contains the [`cr0 flags`] (bits 3-4)
   ///
   /// [`cr0 flags`]: <https://wiki.osdev.org/CPU_Registers_x86#CR0>
   #[derive(PartialEq, Eq, Debug, Clone, Copy)]
   pub struct Cr0: u64 {
       /// If 1, system is in protected mode, else, system is in real mode
       const ProtectedModeEnable = 1 << 0;
       /// Controls interaction of WAIT/FWAIT instructions with TS flag in CR0
       const MonitorCoProcessor = 1 << 1;
       /// If set, no x87 floating-point unit present, if clear, x87 FPU present
       const Emulation = 1 << 2;
       /// Allows saving x87 task context upon a task switch only after x87 instruction used
       const TaskSwitched = 1 << 3;
       /// On the 386, it allowed to specify whether the external math coprocessor was an 80287 or 80387
       const ExtensionType = 1 << 4;
       /// On the 486 and later, enable internal x87 floating point error reporting when set, else enable PC-style error reporting from the internal floating-point unit using external logic
       const NumbericError = 1 << 5;
       /// When set, the CPU cannot write to read-only pages when privilege level is 0
       const WriteProtect = 1 << 16;
       /// Alignment check enabled if AM set, AC flag (in EFLAGS register) set, and privilege level is 3
       const AlignmentMask = 1 << 18;
       /// Globally enables/disable write-through caching
       const NotWriteThrough = 1 << 29;
       /// Globally enables/disable the memory cache
       const CacheDisable = 1 << 30;
       /// If 1, enable paging and use the CR3 register, else disable paging.
       const Paging = 1 << 31;
   }
}

impl Cr0 {
    /// Read from the cr0 into flags
    #[inline(always)]
    pub fn read() -> Self {
        let result: u64;
        // SAFETY: We reading the cr0 is safe we're not setting it
        unsafe {
            asm!("mov {}, cr0", out(reg) result, options(nostack, preserves_flags));
        }

        Self::from_bits_truncate(result)
    }

    /// Write to the register but retain the origial bit if it was set
    ///
    /// # Safety
    ///
    /// the caller must ensure that the provided flags does not cause any unsafe side effects,
    /// or unset the flags that keep the system running
    #[inline(always)]
    pub unsafe fn write_retained(self) {
        unsafe {
            asm!("mov cr0, {}", in(reg) (Self::read() | self).bits(), options(nostack, preserves_flags));
        }
    }

    /// Write the flags into the cr0 literally
    ///
    /// # Safety
    ///
    /// the caller must ensure that the provided flags does not cause any unsafe side effects,
    /// or unset the flags that keep the system running
    #[inline(always)]
    pub unsafe fn write(self) {
        unsafe {
            asm!("mov cr0, {}", in(reg) self.bits(), options(nostack, preserves_flags));
        }
    }
}

pub struct CS;

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

pub struct SS;

impl SS {
    /// Read from the ss segment register
    #[inline(always)]
    pub fn read() -> SegmentSelector {
        let result: u16;
        // SAFETY: We reading the ss is safe we're not setting it
        unsafe {
            asm!("mov {0:x}, ss", out(reg) result, options(nostack, nomem, preserves_flags));
        }

        SegmentSelector(result)
    }

    #[inline(always)]
    pub unsafe fn set(sel: SegmentSelector) {
        unsafe {
            asm!("mov ss, {0:x}", in(reg) sel.0);
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct SegmentSelector(pub u16);

impl SegmentSelector {
    pub fn new(index: u16, rpl: PrivilegeLevel) -> Self {
        Self(index << 3 | rpl.as_u16())
    }

    pub fn index(&self) -> u16 {
        self.0 >> 3
    }

    pub fn privilege_level(&self) -> PrivilegeLevel {
        PrivilegeLevel::from_u16_truncate(self.0)
    }
}

#[derive(Debug)]
pub struct Msr(u32);

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

bitflags! {
    /// Contains the [`efer flags`] (bits 3-4)
    ///
    /// [`efer flags`]: <https://wiki.osdev.org/CPU_Registers_x86-64#IA32_EFER>
    /// [`efer flags`]: <https://en.wikipedia.org/wiki/Control_register#EFER>
    #[derive(PartialEq, Eq, Debug, Clone, Copy)]
    pub struct Efer: u64 {
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
}

impl Efer {
    pub const IA32_EFER_MSR: Msr = Msr::new(0xC0000080);

    /// Read from the [`Efer::IA32_EFER_MSR`] truncate the reserved bits
    pub fn read() -> Self {
        Self::from_bits_truncate(unsafe { Self::IA32_EFER_MSR.read() })
    }

    /// Read the efer msr and then perform bitwise or with the current value, and write that value back
    /// if you somehow create an invalid [`EferFlags`] that contains reserved bits, and crash that
    /// on you ¯\_(ツ)_/¯ (happens to me once)
    ///
    /// # Safety
    ///
    /// the caller must ensure that the provided flags does not cause any unsafe side effects
    #[inline(always)]
    pub unsafe fn write_retained(self) {
        unsafe {
            (Self::read() | self).write();
        }
    }
    /// Write the current flags into the msr not preserving any values, if you somehow create an invalid [`EferFlags`]
    /// that contains reserved bits, and crash that on you ¯\_(ツ)_/¯
    ///
    /// # Safety
    ///
    /// the caller must ensure that the provided flags does not cause any unsafe side effects
    #[inline(always)]
    pub unsafe fn write(self) {
        let msr = Self::IA32_EFER_MSR;
        unsafe { msr.write(self.bits()) };
    }
}

bitflags! {
    #[derive(PartialEq, Eq, Debug, Clone, Copy)]
    pub struct Cr4Flags: u64 {
        /// Virtual 8086 Mode Extensions
        const VME = 1 << 0;
        /// Protected-mode Virtual Interrupts
        const PVI = 1 << 1;
        /// Time Stamp Disable
        const TSD = 1 << 2;
        /// Debugging Extensions
        const DE = 1 << 3;
        /// Page Size Extension
        const PSE = 1 << 4;
        /// Physical Address Extension
        const PAE = 1 << 5;
        /// Machine Check Exception
        const MCE = 1 << 6;
        /// Page Global Enabled
        const PGE = 1 << 7;
        /// Performance-Monitoring Counter enable
        const PCE = 1 << 8;
        /// Operating system support for FXSAVE and FXRSTOR instructions
        const OSFXS = 1 << 9;
        /// Operating System Support for Unmasked SIMD Floating-Point Exceptions
        const OSXMMEXCPT = 1 << 10;
        /// User-Mode Instruction Prevention (if set, #GP on SGDT, SIDT, SLDT, SMSW, and STR instructions when CPL > 0)
        const UMIP 	= 1 << 11;
        /// 57-bit linear addresses (if set, the processor uses 5-level paging otherwise it uses uses 4-level paging)
        const LA57 	= 1 << 12;
        /// Virtual Machine Extensions Enable
        const VMXE 	= 1 << 13;
        /// Safer Mode Extensions Enable
        const SMXE 	= 1 << 14;
        /// Enables the instructions RDFSBASE, RDGSBASE, WRFSBASE, and WRGSBASE
        const FSGSBASE 	= 1 << 16;
        /// PCID Enable
        const PCIDE 	= 1 << 17;
        /// XSAVE and Processor Extended States Enable
        const OSXSAVE 	= 1 << 18;
        /// Supervisor Mode Execution Protection Enable
        const SMEP 	= 1 << 20;
        /// Supervisor Mode Access Prevention Enable
        const SMAP 	= 1 << 21;
        /// Protection Key Enable
        const PKE 	= 1 << 22;
        /// Control-flow Enforcement Technology
        const CET 	= 1 << 23;
        /// Enable Protection Keys for Supervisor-Mode Pages
        const PKS 	= 1 << 24;
    }
}

pub struct Cr4;

impl Cr4 {
    /// Read from the cr4 into flags
    #[inline(always)]
    pub fn read() -> Cr4Flags {
        let result: u64;
        // SAFETY: We reading the cr4 is safe we're not setting it
        unsafe {
            asm!("mov {}, cr4", out(reg) result, options(nostack, preserves_flags));
        }

        Cr4Flags::from_bits_truncate(result)
    }

    /// Read the cr4 and then perform bitwise or with the provided flags, and write that value back
    ///
    /// # Safety
    ///
    /// the caller must ensure that the provided flags does not cause any unsafe side effects
    #[inline(always)]
    pub unsafe fn write_or(flags: Cr4Flags) {
        let flags = Self::read() | flags;

        unsafe {
            Cr4::write(flags);
        }
    }

    /// Write the flags into the Cr4 literally
    ///
    /// # Safety
    ///
    /// the caller must ensure that the provided flags does not cause any unsafe side effects,
    /// or unset the flags that keep the system running
    #[inline(always)]
    pub unsafe fn write(flags: Cr4Flags) {
        unsafe {
            asm!("mov cr4, {}", in(reg) flags.bits(), options(nostack, preserves_flags));
        }
    }
}

bitflags! {
    #[derive(PartialEq, Eq, Debug, Clone, Copy)]
    pub struct Xcr0: u64 {
        /// x87 FPU/MMX support (must be 1)
        const X87 = 1 << 0;
        /// XSAVE support for MXCSR and XMM registers
        const SEE = 1 << 1;
        /// AVX enabled and XSAVE support for upper halves of YMM registers
        const AVX = 1 << 2;
        /// MPX enabled and XSAVE support for BND0-BND3 registers
        const BNDREG = 1 << 3;
        /// MPX enabled and XSAVE support for BNDCFGU and BNDSTATUS registers
        const BINDCSR = 1 << 4;
        /// AVX-512 enabled and XSAVE support for opmask registers k0-k7
        const OPMASK = 1 << 5;
        /// AVX-512 enabled and XSAVE support for upper halves of lower ZMM registers
        const ZMM_HIGH256 = 1 << 6;
        /// AVX-512 enabled and XSAVE support for upper ZMM registers
        const HI16_ZMM = 1 << 7;
        const PKRU = 1 << 9;
    }
}

impl Xcr0 {
    #[inline(always)]
    pub fn read() -> Self {
        Self::from_bits_truncate(unsafe { _xgetbv(0) })
    }

    #[inline(always)]
    pub unsafe fn write_retained(self) {
        unsafe { (Self::read() | self).write() }
    }

    #[inline(always)]
    pub unsafe fn write(self) {
        unsafe { _xsetbv(0, self.bits()) };
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(PartialEq, Eq, Debug, Clone, Copy)]
    pub struct RFlags: u64 {
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

impl RFlags {
    pub fn read() -> Self {
        let value: u64;
        unsafe {
            asm!("pushfq; pop {:r}", out(reg) value, options(nomem, preserves_flags));
        }

        Self::from_bits_truncate(value)
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

#[inline(always)]
pub unsafe fn lgdt(gdt: &DescriptorTablePointer) {
    unsafe {
        asm!("lgdt [{}]", in(reg) gdt, options(readonly, nostack, preserves_flags));
    }
}

#[inline(always)]
pub fn sgdt() -> DescriptorTablePointer {
    let result = DescriptorTablePointer {
        base: VirtAddr::null(),
        limit: 0,
    };

    unsafe {
        asm!("sgdt [{}]", in(reg) &result, options(readonly, nostack, preserves_flags));
    }

    result
}

#[inline(always)]
pub unsafe fn load_tss(selector: SegmentSelector) {
    unsafe {
        asm!("ltr {0:x}", in(reg) selector.0, options(nostack, preserves_flags));
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

    /// Same as [Cr3::reload]
    pub fn full_flush() {
        Cr3::reload();
    }
}
