use bootbridge::BootBridge;

use crate::initialization_context::{InitializationContext, Phase3};

pub mod acpi;
pub mod display;
pub mod pci;
pub mod pit;
pub mod uefi_runtime;

pub fn init(ctx: &mut InitializationContext<Phase3>) {
    pci::init(ctx);
}
