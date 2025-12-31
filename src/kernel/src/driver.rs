use crate::initialization_context::{InitializationContext, Stage4};

pub mod acpi;
pub mod display;
pub mod pci;
pub mod pit;
pub mod uefi_runtime;

pub fn init(ctx: &mut InitializationContext<Stage4>) {
    pci::init(ctx);
}
