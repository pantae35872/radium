use crate::initialization_context::{End, InitializationContext};

pub mod pipeline;
mod syscall;

pub fn init(ctx: &mut InitializationContext<End>) {
    pipeline::init(ctx);
}
