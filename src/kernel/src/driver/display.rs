use crate::initialization_context::{InitializationContext, Phase1};

pub mod vga;

pub fn init(ctx: &mut InitializationContext<Phase1>) {
    vga::init(ctx);
}
