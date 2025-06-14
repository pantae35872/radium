use core::{marker::PhantomData, ops::RangeBounds};

use bit_field::BitField;
use pager::{address::VirtAddr, registers::Msr};

use super::{ApicMode, IA32_APIC_BASE_MSR, X2APIC_BASE_MSR};

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
                pub struct [< ApicReg $name >];

                $( unsafe impl [< LocalApicRegister $mode >] for [< ApicReg $name >] {} )*
            )*

            pub struct BasedApic;

            unsafe impl LocalApicRegisterWrite for BasedApic {}
            unsafe impl LocalApicRegisterRead for BasedApic {}

            pub struct ApicRegisters {
                pub base: LocalApicRegister<BasedApic>,
                $(
                    pub $name: LocalApicRegister<[< ApicReg $name >]>,
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

pub enum LocalApicRegister<T> {
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
    pub fn write(&mut self, value: usize)
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
    pub fn read(&self) -> usize {
        match self {
            Self::X2Apic { msr, .. } => unsafe { msr.read() as usize },
            Self::Apic { addr, .. } => unsafe { *addr.as_mut_ptr::<u32>() as usize },
        }
    }

    pub fn read_bits(&self, range: impl RangeBounds<usize>) -> usize {
        self.read().get_bits(range)
    }

    pub fn read_bit(&self, bit: usize) -> bool {
        self.read().get_bit(bit)
    }
}

impl<T: LocalApicRegisterWrite + LocalApicRegisterRead> LocalApicRegister<T> {
    pub fn write_bits(&mut self, range: impl RangeBounds<usize>, value: usize) -> usize {
        let bits = *self.read().set_bits(range, value);
        self.write(bits);
        bits
    }

    pub fn write_bit(&mut self, bit: usize, value: bool) -> usize {
        let bits = *self.read().set_bit(bit, value);
        self.write(bits);
        bits
    }
}

pub unsafe trait LocalApicRegisterRead {}
pub unsafe trait LocalApicRegisterWrite {}
