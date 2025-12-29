use crate::userland::pipeline::{CommonRequestContext, ControlPipeline, PipelineContext};

#[derive(Debug, Clone, Copy)]
pub struct SyscallId(u32);

enum Syscall {
    Exit,
    Sleep,
}

impl TryFrom<SyscallId> for Syscall {
    type Error = u32;

    fn try_from(value: SyscallId) -> Result<Self, Self::Error> {
        match value {
            SyscallId(0) => Ok(Self::Exit),
            SyscallId(1) => Ok(Self::Sleep),
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
    let calling_task = pipeline_context.interrupted_task.unwrap();

    match syscall {
        Syscall::Exit => pipeline.free_process(calling_task.process),
        Syscall::Sleep => pipeline.sleep_task(calling_task, rq_context.stack_frame.rax as usize),
    }
}
