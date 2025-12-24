//! This module manages all sorts of pipeline[^pipeline] that will be executed in an event of a request
//! either from a user syscall or a hardware interrupt, returning the [`Dispatcher`] as a result.
//!
//! **Use** [`handle_request`] **function to handle the request, the return value will be the**
//! [`Dispatcher`].
//!
//! # Implementaion details
//!
//! The main data structure of this module is the [`ControlPipeline`] (a cpu local), it contains all
//! of the pipeline[^pipeline] that is to be executed when a request comes in. the main idea is that we have a request
//! independent state (the pipeline[^pipeline] stores in [`ControlPipeline`]), and rd[^rd] state
//! [`PipelineContext`].
//!
//! When a request comes in the [`handle_request`] function will create an rd[^rd] context, that context can be used to call
//! different functions within the [`ControlPipeline`] to operate on that context, returning a
//! result depending to that context. This creates a clean seperation between specific request.
//!
//! **Use** [`ControlPipeline::create_context`] to create a context that is rd[^rd].
//!
//!
//! # Definitions
//! [^rd]: referring to a request dependent state
//! [^pipeline]: a request independent procedure managing different type of resources (e.g. thread resources, process resources, ..).

use alloc::vec::Vec;
use kernel_proc::{def_local, local_builder};

use crate::{
    initialization_context::{InitializationContext, Stage4},
    interrupt::{ExtendedInterruptStackFrame, InterruptIndex},
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

pub fn init(ctx: &mut InitializationContext<Stage4>) {
    ctx.local_initializer(|i| {
        i.register(|builder, _ctx, _id| {
            local_builder!(builder, PIPELINE(ControlPipeline::new()));
        })
    });
}

def_local!(pub static PIPELINE: crate::userland::pipeline::ControlPipeline);

/// A cpu local structure that contains all the specific request independent pipelines (the state is shared
/// across request), use [`ControlPipeline::create_context`] to create the specific request
/// context.
pub struct ControlPipeline {
    thread: ThreadPipeline,
    process: ProcessPipeline,
    scheduler: SchedulerPipeline,
}

#[derive(Debug, Default)]
pub struct PipelineContext {
    interrupted_task: Option<TaskBlock>,
    added_tasks: Vec<TaskBlock>,
    added_processes: Vec<Process>,
    should_schedule: bool,
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

    fn handle_ipp(&mut self, context: &mut PipelineContext) {
        self.thread.handle_ipp();
        context.should_schedule = false;
    }

    fn handle_syscall(&mut self, _context: &mut PipelineContext) {}

    fn schedule(&mut self, context: &mut PipelineContext) {
        if !context.should_schedule {
            return;
        }

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
/// about (an indirect reference)
#[derive(Debug, Clone, Copy)]
struct TaskBlock {
    thread: Thread,
    process: Process,
}

/// A structure describing the context of the requester.
pub struct CommonRequestContext<'a> {
    stack_frame: &'a ExtendedInterruptStackFrame,
    referer: RequestReferer,
}

impl<'a> CommonRequestContext<'a> {
    pub fn new(stack_frame: &'a ExtendedInterruptStackFrame, referer: RequestReferer) -> Self {
        Self {
            stack_frame,
            referer,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum RequestReferer {
    HardwareInterrupt(InterruptIndex),
    SyscallRequest(SyscallId),
}

/// Handle the request with the provided [`CommonRequestContext`], returning a dispatcher
/// [`Dispatcher`] that must be used to operate the right following actions.
#[must_use]
pub fn handle_request(rq_context: CommonRequestContext<'_>) -> Dispatcher {
    let pipeline = PIPELINE.inner_mut();
    let mut context = pipeline.create_context(&rq_context);
    match rq_context.referer {
        RequestReferer::SyscallRequest(..) => pipeline.handle_syscall(&mut context),
        RequestReferer::HardwareInterrupt(InterruptIndex::CheckIPP) => {
            pipeline.handle_ipp(&mut context)
        }
        RequestReferer::HardwareInterrupt(i) => {
            todo!("Handle {i:?} hardware interrupt in the scheduler")
        }
    }
    pipeline.schedule(&mut context);
    Dispatcher::new(context)
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

    fn empty() -> Self {
        Self::default()
    }
}

// TODO: Implement Extened States (XSAVE, https://www.felixcloutier.com/x86/xsave)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtendedState;
