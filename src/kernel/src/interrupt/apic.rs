use core::{marker::PhantomData, ops::RangeBounds, usize};

use bit_field::BitField;
use raw_cpuid::CpuId;
use x86_64::registers::model_specific::Msr;

use crate::{inline_if, log, memory::MMIODevice};

use super::InterruptIndex;

pub const IA32_APIC_BASE_MSR: u32 = 0x1B;
pub const X2APIC_BASE_MSR: u32 = 0x800;

// Represent either apic or x2apic
pub struct LocalApic {
    x2apic: bool,
    error_vector: InterruptIndex,
    timer_vector: InterruptIndex,
    spurious_vector: InterruptIndex,
    registers: Option<ApicRegisters>,
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

enum ApicMode {
    X2Apic,
    Apic { base: u64 },
}

enum LocalApicRegister<T> {
    X2Apic { msr: Msr, _phantom: PhantomData<T> },
    Apic { addr: u64, _phantom: PhantomData<T> },
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
                *(*addr as *mut u32) = value as u32;
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
            Self::Apic { addr, .. } => unsafe { *(*addr as *const u32) as usize },
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

impl LocalApic {
    pub fn new(
        timer_vector: InterruptIndex,
        error_vector: InterruptIndex,
        spurious_vector: InterruptIndex,
    ) -> Self {
        let x2apic = CpuId::new().get_feature_info().unwrap().has_x2apic();

        Self {
            error_vector,
            spurious_vector,
            timer_vector,
            x2apic,
            registers: None,
        }
    }

    fn registers_mut(&mut self) -> &mut ApicRegisters {
        self.registers.as_mut().expect("APIC Registers not mapped")
    }

    fn registers(&self) -> &ApicRegisters {
        self.registers.as_ref().expect("APIC Registers not mapped")
    }

    pub fn id(&self) -> usize {
        inline_if!(
            self.x2apic,
            self.registers().id.read(),
            self.registers().id.read_bits(24..32)
        )
    }

    pub fn enable(&mut self) {
        if self.x2apic {
            self.registers_mut().base.write_bit(10, true);
        }
        log!(Info, "Enabling local apic for apic id: {}", self.id());
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
        self.registers_mut()
            .lvt_timer
            .write_bits(0..8, timer_vector);
        self.registers_mut()
            .lvt_error
            .write_bits(0..8, error_vector);
        self.registers_mut()
            .spurious_interrupt
            .write_bits(0..8, spurious_vector);

        self.disable_local_interrupt_pins();

        self.software_enable();
    }

    pub fn start_timer(&mut self, initial_count: usize, divide: TimerDivide, mode: TimerMode) {
        self.registers_mut()
            .divide_configuration
            .write_bits(0..4, divide as u8 as usize);
        self.registers_mut()
            .lvt_timer
            .write_bits(17..19, mode as u8 as usize);
        self.registers_mut().initial_count.write(initial_count);
    }

    pub fn enable_timer(&mut self) {
        self.registers_mut().lvt_timer.write_bit(16, false);
    }

    pub fn disable_timer(&mut self) {
        self.registers_mut().lvt_timer.write_bit(16, true);
    }

    fn software_enable(&mut self) {
        self.registers_mut().spurious_interrupt.write_bit(8, true);
    }

    fn write_icr(&mut self, value: u64) {
        if self.x2apic {
            log!(Debug, "Writing ICR: {:#x}", value);
            unsafe { Msr::new(0x830).write(value) };
        } else {
            self.registers_mut()
                .icr_high
                .write((value as usize >> 32) & 0xFFFFFFFF);
            log!(Trace, "Writing icr: {:#x}", value & 0xFFFFFFFF);
            self.registers_mut()
                .icr_low
                .write(value as usize & 0xFFFFFFFF);
            while self.registers().icr_low.read_bit(12) {
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
        self.registers_mut().lvt_lint0.write(0);
        self.registers_mut().lvt_lint1.write(0);
    }

    pub fn eoi(&mut self) {
        self.registers_mut().eoi.write(0);
    }
}

fn lapic_base() -> u64 {
    unsafe {
        x86_64::registers::model_specific::Msr::new(IA32_APIC_BASE_MSR).read() & 0xFFFFFF000 as u64
    }
}

impl MMIODevice for LocalApic {
    fn start(&self) -> Option<u64> {
        if self.x2apic {
            None
        } else {
            Some(lapic_base())
        }
    }
    fn page_count(&self) -> Option<usize> {
        if self.x2apic {
            None
        } else {
            Some(1)
        }
    }

    fn mapped(&mut self, vaddr: Option<u64>) {
        let mode;
        if let Some(vaddr) = vaddr {
            log!(Info, "using apic");
            mode = ApicMode::Apic { base: vaddr };
        } else if self.x2apic {
            log!(Info, "X2APIC capability found, using x2Apic");
            mode = ApicMode::X2Apic;
        } else {
            panic!("Failed to map mmio for apic");
        }
        self.registers = Some(ApicRegisters::new(&mode))
    }
}
