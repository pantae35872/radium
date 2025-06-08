use crate::initialization_context::{End, InitializationContext};

pub mod acpi;
pub mod display;
pub mod pci;
pub mod pit;
pub mod uefi_runtime;

pub fn init(ctx: &mut InitializationContext<End>) {
    pci::init(ctx);
}
