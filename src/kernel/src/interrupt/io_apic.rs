use core::{cmp::Ordering, u8};

use alloc::vec::Vec;
use bit_field::BitField;
use pager::address::VirtAddr;

use crate::{
    driver::acpi::{
        self,
        madt::{MpsINTIFlags, MpsINTIPolarity, MpsINTITriggerMode},
    },
    initialization_context::{InitializationContext, Phase3},
    log,
    memory::{MMIOBuffer, MMIOBufferInfo, MMIODevice},
    utils::VolatileCell,
};

use super::InterruptIndex;

pub struct IoApicManager {
    io_apics: Vec<IoApic>,
    sources_override: [Option<IoApicSourceOverride>; u8::MAX as usize],
}

pub struct IoApicSourceOverride {
    gsi: usize,
    polarity_override: PinPolarity,
    trigger_mode_override: TriggerMode,
}

struct IoApic {
    gsi_base: usize,
    registers: IoApicRegisters,
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum DeliveryMode {
    Fixed = 0x0,
    LowPrioity = 0b01,
    SMI = 0b10,
    NMI = 0b100,
    INIT = 0b101,
    ExtINT = 0b111,
}

#[derive(Debug)]
pub enum Destination {
    PhysicalDestination(usize),
    LogicalDestination,
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum PinPolarity {
    ActiveHigh = 0,
    ActiveLow = 1,
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum TriggerMode {
    Edge = 0,
    Level = 1,
}

struct IoApicRegisters {
    base: u64,
    id: IoApicRegister,
    ver: IoApicRegister,
    arb: IoApicRegister,
}

struct IoApicRegister {
    base: u64,
    reg: u32,
}

#[repr(C)]
struct RawRedirectionTableEntry {
    low: IoApicRegister,
    high: IoApicRegister,
}

#[derive(Debug)]
pub struct RedirectionTableEntry {
    vector: InterruptIndex,
    delivery_mode: DeliveryMode,
    destination: Destination,
    pin_polarity: PinPolarity,
    trigger_mode: TriggerMode,
}

impl Default for TriggerMode {
    fn default() -> Self {
        Self::Edge
    }
}

impl Default for PinPolarity {
    fn default() -> Self {
        Self::ActiveHigh
    }
}

impl RedirectionTableEntry {
    pub fn new(vector: InterruptIndex, apic_id: usize) -> Self {
        Self {
            vector,
            delivery_mode: DeliveryMode::Fixed,
            destination: Destination::PhysicalDestination(apic_id),
            pin_polarity: PinPolarity::default(),
            trigger_mode: TriggerMode::default(),
        }
    }
}

impl IoApicManager {
    pub fn new() -> Self {
        Self {
            io_apics: Vec::new(),
            sources_override: [const { None }; u8::MAX as usize],
        }
    }

    pub fn add_io_apic(
        &mut self,
        base: MMIOBufferInfo,
        gsi_base: usize,
        ctx: &mut InitializationContext<Phase3>,
    ) {
        log!(Debug, "Found IoApic at: {:#x}", base.addr());
        let io_apic = ctx
            .mmio_device::<IoApic, _>(gsi_base, Some(base))
            .expect("Failed to create some io apic");
        let io_apic_max = io_apic.registers.max_redirection_entry();
        let io_apic_gsi_ranges = gsi_base..io_apic_max;
        log!(
            Debug,
            "IoApic GSI Ranges [{}-{}]",
            io_apic_gsi_ranges.start,
            io_apic_gsi_ranges.end
        );
        self.io_apics.push(io_apic);
    }

    pub fn add_source_override(
        &mut self,
        source_override: &acpi::madt::IoApicInterruptSourceOverride,
    ) {
        self.sources_override[source_override.irq_source() as usize] = Some(source_override.into());
    }

    pub fn redirect_legacy_irqs(&mut self, legacy_irq: u8, mut entry: RedirectionTableEntry) {
        let source_override = match &self.sources_override[legacy_irq as usize] {
            Some(source_override) => source_override,
            None => {
                log!(
                    Error,
                    "Couldn't redirect legacy irq {legacy_irq}, to vector {:?}",
                    entry.vector
                );
                return;
            }
        };
        log!(
            Debug,
            "Mapping legacy irq: {legacy_irq} to GSI: {}, to Interrupt vector: {:?}",
            source_override.gsi,
            entry.vector,
        );
        let selected_apic = self
            .io_apics
            .binary_search_by(|item| {
                if source_override.gsi < item.gsi_base {
                    Ordering::Greater
                } else if source_override.gsi
                    >= item.gsi_base + item.registers.max_redirection_entry()
                {
                    Ordering::Less
                } else {
                    Ordering::Equal
                }
            })
            .ok()
            .map(|e| &mut self.io_apics[e])
            .expect("No compatible apic found for legacy irq");
        entry.pin_polarity = source_override.polarity_override;
        entry.trigger_mode = source_override.trigger_mode_override;
        selected_apic.redirect(entry, source_override.gsi);
    }
}

impl From<&acpi::madt::IoApicInterruptSourceOverride> for IoApicSourceOverride {
    fn from(value: &acpi::madt::IoApicInterruptSourceOverride) -> Self {
        Self {
            gsi: value.gsi() as usize,
            polarity_override: value.flags().into(),
            trigger_mode_override: value.flags().into(),
        }
    }
}

impl From<MpsINTIFlags> for TriggerMode {
    fn from(value: MpsINTIFlags) -> Self {
        match value.trigger_mode() {
            MpsINTITriggerMode::Conforms => Self::default(),
            MpsINTITriggerMode::EdgeTriggered => Self::Edge,
            MpsINTITriggerMode::LevelTriggered => Self::Level,
            _ => unreachable!(),
        }
    }
}

impl From<MpsINTIFlags> for PinPolarity {
    fn from(value: MpsINTIFlags) -> Self {
        match value.polarity() {
            MpsINTIPolarity::Conforms => Self::default(),
            MpsINTIPolarity::ActiveHigh => Self::ActiveHigh,
            MpsINTIPolarity::ActiveLow => Self::ActiveLow,
            _ => unreachable!(),
        }
    }
}

impl IoApic {
    fn new(base: MMIOBuffer, gsi_base: usize) -> Self {
        Self {
            gsi_base,
            registers: unsafe { IoApicRegisters::new(base.base()) },
        }
    }

    /// Provide the abslute index of the gsi not the, reletive index to this IoApic
    /// it's is being calculate internally
    fn redirect(&mut self, entry: RedirectionTableEntry, gsi: usize) {
        let reletive_gsi = gsi - self.gsi_base;
        let mut raw_entry = self.registers.redirection_index(reletive_gsi);
        log!(Trace, "Redirecting gsi {:?} to {:?}", gsi, entry);
        raw_entry.redirect(&entry);
        raw_entry.unmask();
    }
}

impl IoApicRegister {
    pub unsafe fn new(base: u64, reg: u32) -> Self {
        Self { base, reg }
    }

    pub fn write(&mut self, value: u32) {
        let io_apic =
            unsafe { core::slice::from_raw_parts_mut(self.base as *mut VolatileCell<u32>, 5) };
        io_apic[0].set(self.reg & 0xFF);
        io_apic[4].set(value);
    }

    pub fn read(&self) -> u32 {
        let io_apic =
            unsafe { core::slice::from_raw_parts_mut(self.base as *mut VolatileCell<u32>, 5) };
        io_apic[0].set(self.reg & 0xFF);
        return io_apic[4].get();
    }
}

impl IoApicRegisters {
    pub unsafe fn new(base: VirtAddr) -> Self {
        Self {
            base: base.as_u64(),
            id: unsafe { IoApicRegister::new(base.as_u64(), 0x00) },
            ver: unsafe { IoApicRegister::new(base.as_u64(), 0x01) },
            arb: unsafe { IoApicRegister::new(base.as_u64(), 0x02) },
        }
    }

    pub fn id(&self) -> usize {
        self.id.read().get_bits(24..28) as usize
    }

    pub fn max_redirection_entry(&self) -> usize {
        self.ver.read().get_bits(16..24) as usize
    }

    pub fn version(&self) -> usize {
        self.ver.read().get_bits(0..8) as usize
    }

    pub fn redirection_index(&mut self, gsi: usize) -> RawRedirectionTableEntry {
        assert!(gsi <= self.max_redirection_entry()); // the bound is inclusive
        RawRedirectionTableEntry {
            low: unsafe { IoApicRegister::new(self.base, 0x10 + gsi as u32 * 2) },
            high: unsafe { IoApicRegister::new(self.base, 0x10 + gsi as u32 * 2 + 1) },
        }
    }
}

impl RawRedirectionTableEntry {
    pub fn redirect(&mut self, entry: &RedirectionTableEntry) {
        let mut low = self.low.read();
        let mut high = self.high.read();
        low.set_bits(0..8, entry.vector.as_usize() as u32);
        low.set_bits(8..11, entry.delivery_mode as u8 as u32);
        low.set_bit(
            11,
            match entry.destination {
                Destination::LogicalDestination => true,
                Destination::PhysicalDestination(_) => false,
            },
        );
        low.set_bit(13, entry.pin_polarity as u8 != 0);
        low.set_bit(15, entry.trigger_mode as u8 != 0);
        high.set_bits(
            24..32,
            match entry.destination {
                Destination::LogicalDestination => {
                    panic!("Unsupported destination mode in IOAPIC")
                }
                Destination::PhysicalDestination(e) => e as u32,
            },
        );
        self.low.write(low);
        self.high.write(high);
    }

    pub fn mask(&mut self) {
        self.low.write(*self.low.read().set_bit(16, true));
    }

    pub fn unmask(&mut self) {
        self.low.write(*self.low.read().set_bit(16, false));
    }

    pub fn status(&self) -> bool {
        self.low.read().get_bit(12)
    }
}

impl MMIODevice<usize> for IoApic {
    fn new(buffer: crate::memory::MMIOBuffer, gsi_base: usize) -> Self {
        Self::new(buffer, gsi_base)
    }
}
