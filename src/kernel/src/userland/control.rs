use pager::paging::{ActivePageTable, table::RecurseLevel4};

use crate::{
    initialization_context::{End, InitializationContext},
    interrupt::{FullInterruptStackFrame, InterruptIndex},
    userland::{control::dispatch::Dispatcher, syscall::SyscallId},
};

mod dispatch;
mod scheduler;

struct ControlPipeline {}

struct TaskBlock {}

pub struct CommonRequestContext<'a> {
    stack_frame: &'a mut FullInterruptStackFrame,
    page_table: ActivePageTable<RecurseLevel4>,
    referer: RequestReferer,
}

#[derive(Debug, Clone, Copy)]
pub enum RequestReferer {
    HardwareInterrupt(InterruptIndex),
    SyscallRequest(SyscallId),
}

#[must_use]
#[derive(Debug)]
enum ResponseAction {
    Continue,
    Dispatch(Dispatcher),
}

pub fn handle_request(context: CommonRequestContext<'_>) -> ResponseAction {
    todo!()
}

pub fn init(ctx: &mut InitializationContext<End>) {
    ctx.local_initializer(|i| {
        i.register(|builder, ctx, id| todo!("Control pipeline initialized here"))
    });
}
