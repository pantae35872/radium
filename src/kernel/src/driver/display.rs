use crate::initialization_context::{InitializationContext, Stage1};

pub mod vga;

pub fn init(ctx: &mut InitializationContext<Stage1>) {
    vga::init(ctx);
}
