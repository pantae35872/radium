use core::{marker::PhantomData, ops::RangeBounds, usize};

use bit_field::BitField;
use pager::address::{PhysAddr, VirtAddr};
use raw_cpuid::CpuId;
use x86_64::registers::model_specific::Msr;

use crate::{
    inline_if, log,
    memory::{MMIOBuffer, MMIOBufferInfo, MMIODevice},
};

use super::InterruptIndex;

pub const IA32_APIC_BASE_MSR: u32 = 0x1B;
pub const X2APIC_BASE_MSR: u32 = 0x800;

// Represent either apic or x2apic
pub struct LocalApic {
    x2apic: bool,
    error_vector: InterruptIndex,
    timer_vector: InterruptIndex,
    spurious_vector: InterruptIndex,
    mode: ApicMode,
    registers: ApicRegisters,
}

macro_rules! apic_registers {
    (
        $(
            $name:ident($( $mode:ident )|*) : $offset:expr
        ),*
        $(,)?
    ) => {
        paste::paste! {
            $(
                #[allow(non_camel_case_types)]
                struct [< ApicReg $name >];

                $( unsafe impl [< LocalApicRegister $mode >] for [< ApicReg $name >] {} )*
            )*

            struct BasedApic;

            unsafe impl LocalApicRegisterWrite for BasedApic {}
            unsafe impl LocalApicRegisterRead for BasedApic {}

            struct ApicRegisters {
                base: LocalApicRegister<BasedApic>,
                $(
                    $name: LocalApicRegister<[< ApicReg $name >]>,
                )*
            }

            impl ApicRegisters {
                pub fn new(mode: &ApicMode) -> Self {
                    Self {
                        base: LocalApicRegister::<BasedApic>::new_base(),
                        $(
                            $name: LocalApicRegister::<[< ApicReg $name >]>::from_mode(mode, $offset),
                        )*
                    }
                }
            }
        }
    };
}

apic_registers! {
    id(Read): 0x20,
    version(Read): 0x30,
    tpr(Read|Write): 0x80,
    ppr(Read): 0xA0,
    eoi(Write): 0xB0,
    rrd(Read): 0xC0,
    logical_destination(Read): 0xD0,
    destination_format(Read|Write): 0xE0,
    spurious_interrupt(Read|Write): 0xF0,
    isr_0(Read): 0x100,
    isr_1(Read): 0x110,
    isr_2(Read): 0x120,
    isr_3(Read): 0x130,
    isr_4(Read): 0x140,
    isr_5(Read): 0x150,
    isr_6(Read): 0x160,
    isr_7(Read): 0x170,
    tmr_0(Read): 0x180,
    tmr_1(Read): 0x190,
    tmr_2(Read): 0x1A0,
    tmr_3(Read): 0x1B0,
    tmr_4(Read): 0x1C0,
    tmr_5(Read): 0x1D0,
    tmr_6(Read): 0x1E0,
    tmr_7(Read): 0x1F0,
    irr_0(Read): 0x200,
    irr_1(Read): 0x210,
    irr_2(Read): 0x220,
    irr_3(Read): 0x230,
    irr_4(Read): 0x240,
    irr_5(Read): 0x250,
    irr_6(Read): 0x260,
    irr_7(Read): 0x270,
    err_status(Read|Write): 0x280,
    cmci(Read|Write): 0x2F0,
    icr_low(Read|Write): 0x300,
    icr_high(Read|Write): 0x310,
    lvt_timer(Read|Write): 0x320,
    lvt_thermal_sensor(Read|Write): 0x330,
    lvt_performance_monitoring(Read|Write): 0x340,
    lvt_lint0(Read|Write): 0x350,
    lvt_lint1(Read|Write): 0x360,
    lvt_error(Read|Write): 0x370,
    initial_count(Read|Write): 0x380,
    current_count(Read|Write): 0x390,
    divide_configuration(Read|Write): 0x3E0,
}

#[derive(Clone)]
enum ApicMode {
    X2Apic,
    Apic { base: VirtAddr },
}

enum LocalApicRegister<T> {
    X2Apic {
        msr: Msr,
        _phantom: PhantomData<T>,
    },
    Apic {
        addr: VirtAddr,
        _phantom: PhantomData<T>,
    },
}

impl<T> LocalApicRegister<T> {
    fn new_base() -> Self {
        Self::X2Apic {
            msr: Msr::new(IA32_APIC_BASE_MSR),
            _phantom: PhantomData,
        }
    }

    fn from_mode(mode: &ApicMode, offset: usize) -> Self {
        match mode {
            ApicMode::X2Apic => Self::X2Apic {
                msr: Msr::new(X2APIC_BASE_MSR + (offset as u32 >> 4)),
                _phantom: PhantomData,
            },
            ApicMode::Apic { base } => Self::Apic {
                addr: *base + offset as u64,
                _phantom: PhantomData,
            },
        }
    }
}

impl<T: LocalApicRegisterWrite> LocalApicRegister<T> {
    /// Write to an apic register
    fn write(&mut self, value: usize)
    where
        T: LocalApicRegisterWrite,
    {
        match (self, value as u64) {
            (Self::X2Apic { msr, .. }, value) => unsafe { msr.write(value) },
            (Self::Apic { addr, .. }, value) if value <= u32::MAX as u64 => unsafe {
                *addr.as_mut_ptr::<u32>() = value as u32;
            },
            _ => panic!("Cannot write value more than u64 to a APIC register"),
        }
    }
}

impl<T: LocalApicRegisterRead> LocalApicRegister<T> {
    /// Read from an apic register
    fn read(&self) -> usize {
        match self {
            Self::X2Apic { msr, .. } => unsafe { msr.read() as usize },
            Self::Apic { addr, .. } => unsafe { *addr.as_mut_ptr::<u32>() as usize },
        }
    }

    fn read_bits(&self, range: impl RangeBounds<usize>) -> usize {
        self.read().get_bits(range)
    }

    fn read_bit(&self, bit: usize) -> bool {
        self.read().get_bit(bit)
    }
}

impl<T: LocalApicRegisterWrite + LocalApicRegisterRead> LocalApicRegister<T> {
    fn write_bits(&mut self, range: impl RangeBounds<usize>, value: usize) -> usize {
        let bits = *self.read().set_bits(range, value);
        self.write(bits);
        bits
    }

    fn write_bit(&mut self, bit: usize, value: bool) -> usize {
        let bits = *self.read().set_bit(bit, value);
        self.write(bits);
        bits
    }
}

unsafe trait LocalApicRegisterRead {}
unsafe trait LocalApicRegisterWrite {}

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum TimerMode {
    OneShot = 0,
    Periodic = 1,
    TscDeadLine = 2,
}

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum TimerDivide {
    Div2 = 0,
    Div4 = 1,
    Div8 = 2,
    Div16 = 3,
    Div32 = 0b1000,
    Div64 = 0b1001,
    Div128 = 0b1010,
    Div1 = 0b1011,
}

#[repr(u8)]
pub enum IpiDestMode {
    Physical = 0,
    Logical = 1,
}

#[derive(Debug)]
#[repr(u8)]
pub enum IpiDeliveryMode {
    Fixed = 0b000,
    LowestPriority = 0b001,
    SystemManagement = 0b010,
    NonMaskable = 0b100,
    Init = 0b101,
    StartUp = 0b110,
}

pub struct LocalApicArguments {
    pub timer_vector: InterruptIndex,
    pub error_vector: InterruptIndex,
    pub spurious_vector: InterruptIndex,
}

impl LocalApic {
    pub fn id(&self) -> usize {
        inline_if!(
            self.x2apic,
            self.registers.id.read(),
            self.registers.id.read_bits(24..32)
        )
    }

    pub fn enable(&mut self) {
        // FIXME: APIC timer interrupts now working on legacy apic mode
        // EDIT:
        // Umm idk if this is fixed or not it's suddenly just started working in apic mode
        // I tried git diff and i really didn't change anything it think it's the problem with
        // qemu, i can't reproduce it now
        if self.x2apic {
            self.registers.base.write_bit(10, true);
        }
        log!(Debug, "Enabling local apic for apic id: {}", self.id());
        let timer_vector = self.timer_vector.as_usize();
        let error_vector = self.error_vector.as_usize();
        let spurious_vector = self.spurious_vector.as_usize();
        log!(
            Trace,
            "Remaping local apic id: {}; Timer vector: {}, Error vector: {}, Spurious vector: {}",
            self.id(),
            timer_vector,
            error_vector,
            spurious_vector
        );
        self.registers.lvt_timer.write_bits(0..8, timer_vector);
        self.registers.lvt_error.write_bits(0..8, error_vector);
        self.registers
            .spurious_interrupt
            .write_bits(0..8, spurious_vector);

        self.disable_local_interrupt_pins();

        self.software_enable();
    }

    pub fn start_timer(&mut self, initial_count: usize, divide: TimerDivide, mode: TimerMode) {
        self.registers
            .divide_configuration
            .write_bits(0..4, divide as u8 as usize);
        self.registers
            .lvt_timer
            .write_bits(17..19, mode as u8 as usize);
        self.registers.initial_count.write(initial_count);
    }

    pub fn enable_timer(&mut self) {
        self.registers.lvt_timer.write_bit(16, false);
    }

    pub fn disable_timer(&mut self) {
        self.registers.lvt_timer.write_bit(16, true);
    }

    fn software_enable(&mut self) {
        self.registers.spurious_interrupt.write_bit(8, true);
    }

    fn write_icr(&mut self, value: u64) {
        if self.x2apic {
            unsafe { Msr::new(0x830).write(value) };
        } else {
            self.registers
                .icr_high
                .write((value as usize >> 32) & 0xFFFFFFFF);
            self.registers.icr_low.write(value as usize & 0xFFFFFFFF);
            while self.registers.icr_low.read_bit(12) {
                core::hint::spin_loop();
            }
        }
    }

    pub fn send_init_ipi(&mut self, destination: usize) {
        let mut icr = 0u64;
        if self.x2apic {
            icr.set_bits(32..64, destination as u64);
        } else {
            icr.set_bits(56..64, destination as u64);
        }
        icr.set_bits(0..8, 0);
        icr.set_bits(8..11, IpiDeliveryMode::Init as u8 as u64);
        icr.set_bit(11, IpiDestMode::Physical as u8 == 1);
        icr.set_bit(14, true);
        icr.set_bit(15, true);
        self.write_icr(icr);
    }

    // NOTE: There two version of this idk which one to use
    // This one is from https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-3a-part-1-manual.pdf
    // The Intel SDM volume 3A section 10.12.10.2
    // Logical x2APIC ID = [(x2APIC ID[19:4] « 16) | (1 « x2APIC ID[3:0])]
    // And this one is from https://courses.cs.washington.edu/courses/cse451/24sp/resources/x2apic.pdf
    // Logical x2APIC ID = [(x2APIC ID[31:4] << 16) | (1 << x2APIC ID[3:0])]
    fn id_to_logical_destination(id: usize) -> u64 {
        ((id.get_bits(4..20) << 16) | (1 << id.get_bits(0..4))) as u64
    }

    pub fn send_startup_ipi(&mut self, destination: usize) {
        let mut icr = 0u64;
        if self.x2apic {
            icr.set_bits(32..64, destination as u64);
        } else {
            icr.set_bits(56..64, destination as u64);
        }
        icr.set_bits(0..8, 0b1000);
        icr.set_bits(8..11, IpiDeliveryMode::StartUp as u8 as u64);
        icr.set_bit(11, IpiDestMode::Physical as u8 == 1);
        icr.set_bit(14, true);
        icr.set_bit(14, false);
        self.write_icr(icr);
    }

    fn disable_local_interrupt_pins(&mut self) {
        log!(
            Trace,
            "Disabling local interrupt pins for apic id: {}",
            self.id()
        );
        self.registers.lvt_lint0.write(0);
        self.registers.lvt_lint1.write(0);
    }

    pub fn eoi(&mut self) {
        self.registers.eoi.write(0);
    }
}

impl Clone for LocalApic {
    fn clone(&self) -> Self {
        Self {
            x2apic: self.x2apic,
            error_vector: self.error_vector,
            timer_vector: self.timer_vector,
            spurious_vector: self.spurious_vector,
            mode: self.mode.clone(),
            registers: ApicRegisters::new(&self.mode),
        }
    }
}

fn lapic_base() -> PhysAddr {
    unsafe {
        PhysAddr::new(
            x86_64::registers::model_specific::Msr::new(IA32_APIC_BASE_MSR).read()
                & 0xFFFFFF000 as u64,
        )
    }
}

impl MMIODevice<LocalApicArguments> for LocalApic {
    fn other() -> Option<crate::memory::MMIOBufferInfo> {
        Some(unsafe { MMIOBufferInfo::new_raw(lapic_base(), 1) })
    }

    fn new(buffer: MMIOBuffer, args: LocalApicArguments) -> Self {
        let LocalApicArguments {
            timer_vector,
            error_vector,
            spurious_vector,
        } = args;
        let x2apic = CpuId::new().get_feature_info().unwrap().has_x2apic();

        let mode = inline_if!(
            x2apic,
            ApicMode::X2Apic,
            ApicMode::Apic {
                base: buffer.base()
            }
        );
        Self {
            error_vector,
            spurious_vector,
            timer_vector,
            x2apic,
            registers: ApicRegisters::new(&mode),
            mode,
        }
    }
}
