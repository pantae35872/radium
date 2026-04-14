use pager::address::VirtAddr;

use crate::{
    logger::LOGGER,
    userland::pipeline::{CommonRequestContext, ControlPipeline, PipelineContext},
};

#[derive(Debug, Clone, Copy)]
pub struct SyscallId(pub u32);

enum Syscall {
    Exit,
    Sleep,
    Spawn,
    ExitThread,
    Test,
    Flush,
}

impl TryFrom<SyscallId> for Syscall {
    type Error = u32;

    fn try_from(value: SyscallId) -> Result<Self, Self::Error> {
        match value {
            SyscallId(0) => Ok(Self::Exit),
            SyscallId(1) => Ok(Self::Sleep),
            SyscallId(2) => Ok(Self::Spawn),
            SyscallId(3) => Ok(Self::ExitThread),
            SyscallId(4) => Ok(Self::Test),
            SyscallId(5) => Ok(Self::Flush),
            SyscallId(unknown) => Err(unknown),
        }
    }
}

pub(super) fn syscall_handle(
    rq_context: &CommonRequestContext,
    pipeline: &mut ControlPipeline,
    pipeline_context: &mut PipelineContext,
    syscall: SyscallId,
) {
    let syscall = Syscall::try_from(syscall).unwrap_or(Syscall::Exit);
    let Some(calling_task) = pipeline_context.interrupted_task else {
        return;
    };

    if !calling_task.valid() {
        return;
    }

    match syscall {
        Syscall::Exit => pipeline.free_process(calling_task.process),
        Syscall::Sleep => pipeline.sleep_interrupted(pipeline_context, rq_context.stack_frame.rdx as usize),
        Syscall::Spawn => {
            let start = VirtAddr::new(rq_context.stack_frame.rdx);
            if pipeline.alloc_thread(pipeline_context, calling_task.process, start).is_none() {
                pipeline.free_process(calling_task.process);
            }
        }
        Syscall::ExitThread => {
            pipeline.free_thread(pipeline_context, calling_task.thread);
        }
        Syscall::Flush => {
            LOGGER.flush_all(&[|s| serial_print!("{s}")]);
        }
        Syscall::Test => {
            serial_print!("{}", char::from_u32(rq_context.stack_frame.rdx.try_into().unwrap_or(0)).unwrap_or('?'));
        }
    }
}
