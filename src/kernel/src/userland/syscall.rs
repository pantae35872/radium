use crate::userland::pipeline::{ControlPipeline, PipelineContext};

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
    pipeline: &mut ControlPipeline,
    pipeline_context: &mut PipelineContext,
    syscall: SyscallId,
) {
    let syscall = Syscall::try_from(syscall).unwrap_or(Syscall::Exit);

    //match syscall {
    //    Syscall::Exit => pipeline_context.exit(),
    //}
}
