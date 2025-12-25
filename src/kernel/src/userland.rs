use pager::address::VirtAddr;

use crate::initialization_context::{InitializationContext, Stage4};

pub const STACK_START: VirtAddr = VirtAddr::new(0x0000_7FFF_0000_0000);
pub const STACK_MAX_SIZE: usize = 0xFFFF_FFFF; // 4 GIB Overall stack per process is probably enough.

pub mod pipeline;
mod syscall;

pub fn init(ctx: &mut InitializationContext<Stage4>) {
    pipeline::init(ctx);
}
