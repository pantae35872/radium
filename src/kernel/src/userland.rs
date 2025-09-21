use crate::initialization_context::{End, InitializationContext};

pub mod control;
mod syscall;

pub fn init(ctx: &mut InitializationContext<End>) {
    control::init(ctx);
}
