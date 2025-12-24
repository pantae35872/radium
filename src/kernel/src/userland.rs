use crate::initialization_context::{InitializationContext, Stage4};

pub mod pipeline;
mod syscall;

pub fn init(ctx: &mut InitializationContext<Stage4>) {
    pipeline::init(ctx);
}
