use bootbridge::BootBridge;

use crate::initialization_context::{FinalPhase, InitializationContext};

pub mod acpi;
pub mod display;
pub mod pci;
pub mod pit;
pub mod uefi_runtime;

pub fn init(ctx: &mut InitializationContext<FinalPhase>) {
    pci::init(ctx);
}
