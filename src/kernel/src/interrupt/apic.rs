use core::{fmt::Display, usize};

use ipi::{IcrBuilder, IpiDeliveryMode, IpiDestination, IpiDestinationShorthand, IpiTriggerMode};
use pager::{
    address::{PhysAddr, VirtAddr},
    registers::Msr,
};
use raw_cpuid::CpuId;
use registers::ApicRegisters;

use crate::{
    driver::pit::PIT,
    inline_if,
    interrupt::TPMS,
    memory::{MMIOBuffer, MMIOBufferInfo, MMIODevice},
    smp::{APIC_ID_TO_CPU_ID, CoreId, core_id_to_apic_id},
};
use sentinel::log;

use super::InterruptIndex;

mod ipi;
mod registers;

pub const IA32_APIC_BASE_MSR: u32 = 0x1B;
pub const X2APIC_BASE_MSR: u32 = 0x800;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct ApicId(usize);

impl ApicId {
    #[inline]
    pub fn id(&self) -> usize {
        self.0
    }

    pub fn new(id: usize) -> Option<Self> {
        if APIC_ID_TO_CPU_ID.get()?.get(id).is_none() {
            return None;
        }
        // SAFETY: We've already validate this above
        Some(unsafe { Self::new_unchecked(id) })
    }

    /// # Safety
    /// The caller must validate or assure that the id is from the valid source
    pub unsafe fn new_unchecked(id: usize) -> Self {
        Self(id)
    }
}

impl Display for ApicId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<CoreId> for ApicId {
    fn from(core_id: CoreId) -> Self {
        Self(core_id_to_apic_id(core_id.id()))
    }
}

impl From<ApicId> for usize {
    fn from(value: ApicId) -> Self {
        value.id()
    }
}

// Represent either apic or x2apic
pub struct LocalApic {
    x2apic: bool,
    error_vector: InterruptIndex,
    timer_vector: InterruptIndex,
    spurious_vector: InterruptIndex,
    mode: ApicMode,
    registers: ApicRegisters,
}

#[derive(Clone)]
enum ApicMode {
    X2Apic,
    Apic { base: VirtAddr },
}

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

pub struct LocalApicArguments {
    pub timer_vector: InterruptIndex,
    pub error_vector: InterruptIndex,
    pub spurious_vector: InterruptIndex,
}

impl LocalApic {
    pub fn id(&self) -> ApicId {
        inline_if!(
            self.x2apic,
            ApicId(self.registers.id.read()),
            ApicId(self.registers.id.read_bits(24..32))
        )
    }

    pub fn current_count(&mut self) -> usize {
        self.registers.current_count.read()
    }

    pub fn calibrate(&mut self) {
        let initial_count = u32::MAX as usize;
        self.start_timer(initial_count, TimerDivide::Div16, TimerMode::OneShot);
        self.enable_timer();

        log!(
            Debug,
            "APIC Timer Count Before PIT 10 ms: {}",
            self.current_count()
        );

        PIT.get().unwrap().lock().dumb_wait_10ms();

        log!(
            Debug,
            "APIC Timer Count After PIT 10 ms: {}",
            self.current_count()
        );
        let ticks_per_ms = (initial_count - self.current_count()) / 10;
        *TPMS.inner_mut() = ticks_per_ms;
        log!(Debug, "Calibrated APIC Timer, TPMS: {ticks_per_ms}");
        self.start_timer(ticks_per_ms, TimerDivide::Div16, TimerMode::OneShot);
    }

    pub fn enable(&mut self) {
        // FIXME: APIC timer interrupts now working on legacy apic mode
        // EDIT:
        // Umm idk if this is fixed or not it's suddenly just started working in apic mode
        // I tried git diff and i really didn't change anything, i think it's the problem with
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

    pub fn reset_timer(&mut self, count: usize) {
        self.registers.initial_count.write(count);
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

    pub fn send_init_ipi(&mut self, destination: impl Into<ApicId>, assertion: bool) {
        let mut builder = IcrBuilder::new(self.x2apic);
        builder
            .assertion(assertion)
            .delivery_mode(IpiDeliveryMode::Init)
            .trigger_mode(IpiTriggerMode::Level)
            .destination(IpiDestination::PhysicalDestination(destination.into()));
        self.write_icr(builder.build().expect("This should be valid"));
    }

    pub fn send_startup_ipi(&mut self, destination: impl Into<ApicId>) {
        let mut builder = IcrBuilder::new(self.x2apic);
        builder
            .vector_raw(0b1000)
            .delivery_mode(IpiDeliveryMode::StartUp)
            .destination(IpiDestination::PhysicalDestination(destination.into()));
        self.write_icr(builder.build().expect("This should be valid"));
    }

    /// Send fixed ipi to all except the current core
    pub fn broadcast_fixed_ipi(&mut self, destination_vector: InterruptIndex) {
        let mut builder = IcrBuilder::new(self.x2apic);
        builder
            .vector(destination_vector)
            .shorthand(IpiDestinationShorthand::AllExcludingSelf)
            .delivery_mode(IpiDeliveryMode::Fixed);
        self.write_icr(builder.build().expect("This should be valid"));
    }

    pub fn send_fixed_ipi(
        &mut self,
        destination: impl Into<ApicId>,
        destination_vector: InterruptIndex,
    ) {
        let mut builder = IcrBuilder::new(self.x2apic);
        builder
            .vector(destination_vector)
            .delivery_mode(IpiDeliveryMode::Fixed)
            .destination(IpiDestination::PhysicalDestination(destination.into()));
        self.write_icr(builder.build().expect("This should be valid"));
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

pub fn apic_id() -> ApicId {
    ApicId(
        CpuId::new()
            .get_feature_info()
            .unwrap()
            .initial_local_apic_id() as usize,
    )
}

fn lapic_base() -> PhysAddr {
    unsafe { PhysAddr::new(Msr::new(IA32_APIC_BASE_MSR).read() & 0xFFFFFF000 as u64) }
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
