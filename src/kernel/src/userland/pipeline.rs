use alloc::vec::Vec;
use pager::paging::{ActivePageTable, table::RecurseLevel4};

use crate::{
    initialization_context::{End, InitializationContext},
    interrupt::{ExtendedInterruptStackFrame, InterruptIndex},
    smp::cpu_local,
    userland::{
        pipeline::{
            dispatch::Dispatcher,
            process::{Process, ProcessPipeline},
            scheduler::SchedulerPipeline,
            thread::{Thread, ThreadPipeline},
        },
        syscall::SyscallId,
    },
};

mod dispatch;
mod process;
mod scheduler;
mod thread;

pub struct ControlPipeline {
    thread: ThreadPipeline,
    process: ProcessPipeline,
    scheduler: SchedulerPipeline,
}

#[derive(Debug, Default)]
struct PipelineContext {
    interrupted_task: Option<TaskBlock>,
    added_tasks: Vec<TaskBlock>,
    added_processes: Vec<Process>,
    scheduled_task: Option<TaskBlock>,
}

impl PipelineContext {
    fn alloc_thread<F>(
        &mut self,
        thread: &mut ThreadPipeline,
        process: &mut ProcessPipeline,
        parent_process: Process,
        start: F,
    ) -> TaskBlock
    where
        F: FnOnce() + Send + 'static,
    {
        let task = thread.alloc(process, parent_process, start);
        self.added_tasks.push(task);
        task
    }

    fn alloc_process(&mut self, process: &mut ProcessPipeline) -> Process {
        let process = process.alloc();
        self.added_processes.push(process);
        process
    }
}

impl ControlPipeline {
    fn new() -> Self {
        Self {
            thread: ThreadPipeline::default(),
            process: ProcessPipeline::default(),
            scheduler: SchedulerPipeline::default(),
        }
    }

    fn create_context(&mut self, context: &CommonRequestContext<'_>) -> PipelineContext {
        let thread = self.thread.sync_and_identify(context);
        let process = self.process.sync_and_identify(context, &thread);
        PipelineContext {
            interrupted_task: Some(TaskBlock { thread, process }),
            ..Default::default()
        }
    }

    fn handle_syscall(&mut self, context: &mut PipelineContext) {}

    fn schedule(&mut self, context: &mut PipelineContext) {
        self.scheduler.schedule(context, &mut self.thread);
    }

    fn thread(&mut self) -> &mut ThreadPipeline {
        &mut self.thread
    }

    fn process(&mut self) -> &mut ProcessPipeline {
        &mut self.process
    }

    fn scheduler(&mut self) -> &mut SchedulerPipeline {
        &mut self.scheduler
    }
}

/// A lightweight struct to store just enough data to know which process or thread, we're talking
/// about
#[derive(Debug, Clone, Copy)]
struct TaskBlock {
    thread: Thread,
    process: Process,
}

pub struct CommonRequestContext<'a> {
    stack_frame: &'a ExtendedInterruptStackFrame,
    page_table: ActivePageTable<RecurseLevel4>,
    referer: RequestReferer,
}

#[derive(Debug, Clone, Copy)]
pub enum RequestReferer {
    HardwareInterrupt(InterruptIndex),
    SyscallRequest(SyscallId),
}

pub fn handle_request(context: CommonRequestContext<'_>) -> Dispatcher {
    let pipeline = cpu_local().pipeline();
    let mut context = pipeline.create_context(&context);
    pipeline.handle_syscall(&mut context);
    pipeline.schedule(&mut context);
    return Dispatcher::new(context);
}

pub fn init(ctx: &mut InitializationContext<End>) {
    ctx.local_initializer(|i| {
        i.register(|builder, ctx, id| {
            builder.control_pipeline(ControlPipeline::new());
        })
    });
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TaskProcesserState {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,

    pub extended_state: ExtendedState,
}

impl TaskProcesserState {
    fn new(context: &CommonRequestContext<'_>) -> Self {
        Self {
            r15: context.stack_frame.r15,
            r14: context.stack_frame.r14,
            r13: context.stack_frame.r13,
            r12: context.stack_frame.r12,
            r11: context.stack_frame.r11,
            r10: context.stack_frame.r10,
            r9: context.stack_frame.r9,
            r8: context.stack_frame.r8,
            rsi: context.stack_frame.rsi,
            rdi: context.stack_frame.rdi,
            rbp: context.stack_frame.rbp,
            rdx: context.stack_frame.rdx,
            rcx: context.stack_frame.rcx,
            rbx: context.stack_frame.rbx,
            rax: context.stack_frame.rax,
            extended_state: ExtendedState,
        }
    }
}

// TODO: Implement Extened States (XSAVE, https://www.felixcloutier.com/x86/xsave)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtendedState;
