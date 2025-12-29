use conquer_once::spin::OnceCell;
use packery::Packed;
use pager::address::VirtAddr;

use crate::{
    initialization_context::{InitializationContext, Stage4},
    initialize_guard,
};

pub const STACK_START: VirtAddr = VirtAddr::new(0x0000_7FFF_0000_0000);
pub const STACK_MAX_SIZE: usize = 0xFFFF_FFFF; // 4 GIB Overall stack per process is probably enough.

pub mod pipeline;
mod syscall;

static PACKED_DATA: OnceCell<Packed<'static>> = OnceCell::uninit();

pub fn init(ctx: &mut InitializationContext<Stage4>) {
    initialize_guard!();

    PACKED_DATA.init_once(|| ctx.context_mut().boot_bridge.packed_programs());

    pipeline::init(ctx);
}
