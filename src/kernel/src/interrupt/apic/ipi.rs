use bit_field::BitField;
use thiserror::Error;

use crate::interrupt::InterruptIndex;

use super::ApicId;

#[derive(Debug, Clone)]
pub struct IcrBuilder {
    delivery_mode: IpiDeliveryMode,
    shorthand: IpiDestinationShorthand,
    vector: Option<u8>,
    assertion: bool,
    dest: Option<IpiDestination>,
    trigger_mode: IpiTriggerMode,
    x2apic: bool,
}

#[derive(Debug, Error)]
pub enum IcrBuilderError {
    #[error(
        "No Shorthand with level trigger mode is invalid, if the delivery mode is SMI, according to the intel specification"
    )]
    NoShorthandWithLevelTriggerAndSystemManagement,
    #[error(
        "Self Shorthand with anything other than fixed delivery mode is invalid, according to the intel specification"
    )]
    ToSelfNotFixed,
    #[error(
        "All including self Shorthand with anything other than fixed delivery mode is invalid, according to the intel specification"
    )]
    AllIncludingSelfNotFixed,
    #[error("Smi and startup with level trigger mode is not valid, according to the intel specification")]
    SMIAndStartUpWithLevel,

    #[error("No shorthand selected but no destination is provided")]
    NoDestinationProvidedWithNoShorthand,
    #[error("Interrupt Vector must not be provided with init delivery mode, according to the intel specification")]
    VectorWithInit,
    #[error("Interrupt Vector is not provided when using other delivery mode other than init delivery mode")]
    NoVectorProvidedNotInit,
    #[error("Assertion must be true when using delivery mode that is not init")]
    NotInitFalseAssert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpiTriggerMode {
    Edge,
    Level,
}

impl From<IpiTriggerMode> for bool {
    fn from(value: IpiTriggerMode) -> Self {
        match value {
            IpiTriggerMode::Edge => false,
            IpiTriggerMode::Level => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IpiDestMode {
    Physical,
    Logical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IpiDestinationShorthand {
    NoShorthand = 0b00,
    ToSelf = 0b01,
    AllIncludingSelf = 0b10,
    AllExcludingSelf = 0b11,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpiDestination {
    PhysicalDestination(ApicId),
    LogicalDestination(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IpiDeliveryMode {
    Fixed = 0b000,
    LowestPriority = 0b001,
    SystemManagement = 0b010,
    NonMaskable = 0b100,
    Init = 0b101,
    StartUp = 0b110,
}

impl IcrBuilder {
    pub const fn new(x2apic: bool) -> Self {
        Self {
            delivery_mode: IpiDeliveryMode::Fixed,
            shorthand: IpiDestinationShorthand::NoShorthand,
            trigger_mode: IpiTriggerMode::Edge,
            vector: None,
            dest: None,
            assertion: true,
            x2apic,
        }
    }

    pub const fn trigger_mode(&mut self, trigger_mode: IpiTriggerMode) -> &mut Self {
        self.trigger_mode = trigger_mode;
        self
    }

    pub const fn assertion(&mut self, assertion: bool) -> &mut Self {
        self.assertion = assertion;
        self
    }

    pub const fn delivery_mode(&mut self, mode: IpiDeliveryMode) -> &mut Self {
        self.delivery_mode = mode;
        self
    }

    pub const fn destination(&mut self, dest: IpiDestination) -> &mut Self {
        self.dest = Some(dest);
        self
    }

    pub const fn shorthand(&mut self, shorthand: IpiDestinationShorthand) -> &mut Self {
        self.shorthand = shorthand;
        self
    }

    pub const fn vector(&mut self, index: InterruptIndex) -> &mut Self {
        self.vector = Some(index.as_u8());
        self
    }

    pub const fn vector_raw(&mut self, vector: u8) -> &mut Self {
        self.vector = Some(vector);
        self
    }

    pub fn build(self) -> Result<u64, IcrBuilderError> {
        let mut icr = 0u64;
        match self {
            Self {
                shorthand: IpiDestinationShorthand::NoShorthand,
                trigger_mode: IpiTriggerMode::Level,
                delivery_mode: IpiDeliveryMode::SystemManagement,
                ..
            } => return Err(IcrBuilderError::NoShorthandWithLevelTriggerAndSystemManagement),
            Self {
                delivery_mode:
                    IpiDeliveryMode::LowestPriority
                    | IpiDeliveryMode::NonMaskable
                    | IpiDeliveryMode::Init
                    | IpiDeliveryMode::SystemManagement
                    | IpiDeliveryMode::StartUp,
                shorthand: IpiDestinationShorthand::ToSelf,
                ..
            } => return Err(IcrBuilderError::ToSelfNotFixed),
            Self {
                shorthand: IpiDestinationShorthand::AllIncludingSelf,
                delivery_mode:
                    IpiDeliveryMode::LowestPriority
                    | IpiDeliveryMode::NonMaskable
                    | IpiDeliveryMode::Init
                    | IpiDeliveryMode::SystemManagement
                    | IpiDeliveryMode::StartUp,
                ..
            } => return Err(IcrBuilderError::AllIncludingSelfNotFixed),
            Self {
                delivery_mode: IpiDeliveryMode::SystemManagement | IpiDeliveryMode::StartUp,
                trigger_mode: IpiTriggerMode::Level,
                ..
            } => return Err(IcrBuilderError::SMIAndStartUpWithLevel),
            Self { delivery_mode, shorthand, trigger_mode, vector, dest, x2apic, assertion } => {
                if !assertion && !matches!(delivery_mode, IpiDeliveryMode::Init) {
                    return Err(IcrBuilderError::NotInitFalseAssert);
                }
                let vector = match (vector, delivery_mode) {
                    (None, IpiDeliveryMode::Init) => 0,
                    (Some(_), IpiDeliveryMode::Init) => {
                        return Err(IcrBuilderError::VectorWithInit);
                    }
                    (Some(vector), _) => vector,
                    (None, _) => return Err(IcrBuilderError::NoVectorProvidedNotInit),
                };
                let (dest, mode) = match (dest, shorthand) {
                    (Some(IpiDestination::LogicalDestination(dest)), IpiDestinationShorthand::NoShorthand) => {
                        (dest as u64, true)
                    }
                    (Some(IpiDestination::PhysicalDestination(dest)), IpiDestinationShorthand::NoShorthand) => {
                        (dest.id() as u64, false)
                    }
                    (
                        _,
                        IpiDestinationShorthand::AllExcludingSelf
                        | IpiDestinationShorthand::ToSelf
                        | IpiDestinationShorthand::AllIncludingSelf,
                    ) => (0, false),
                    (None, IpiDestinationShorthand::NoShorthand) => {
                        return Err(IcrBuilderError::NoDestinationProvidedWithNoShorthand);
                    }
                };
                if x2apic {
                    icr.set_bits(32..64, dest);
                } else {
                    icr.set_bits(56..64, dest);
                }
                icr.set_bits(0..8, vector.into());
                icr.set_bits(8..11, delivery_mode as u8 as u64);
                icr.set_bit(11, mode);
                icr.set_bit(14, assertion);
                icr.set_bit(15, trigger_mode.into());
                icr.set_bits(18..20, shorthand as u8 as u64);
            }
        }
        Ok(icr)
    }
}
