use core::cell::RefCell;

use alloc::{boxed::Box, vec, vec::Vec};
use pager::paging::{ActivePageTable, table::RecurseLevel4};

use crate::{
    initialization_context::{End, InitializationContext},
    interrupt::{ExtendedInterruptStackFrame, InterruptIndex},
    smp::cpu_local,
    userland::{
        control::{
            dispatch::Dispatcher,
            process::{Process, ProcessProcessor},
            scheduler::SchedulerProcessor,
            thread::{Thread, ThreadProcessor},
        },
        syscall::SyscallId,
    },
};

mod dispatch;
mod process;
mod scheduler;
mod thread;

/// Manager of the pipeline
pub struct ControlPipeline {
    pipeline: Vec<Box<dyn TaskProcessor>>,
}

impl ControlPipeline {
    fn new(pipeline: Vec<Box<dyn TaskProcessor>>) -> Self {
        Self { pipeline }
    }

    fn start_processing(&mut self, block: &mut TaskBlock) {
        for pipeline in &mut self.pipeline {
            pipeline.update_task(block);
        }

        for pipeline in &mut self.pipeline {
            pipeline.finalize_update(block);
        }
    }
}

/// A trait respresenting a task processor in the control pipeline
trait TaskProcessor {
    /// This is called when process needs to update (or process) the task block information
    fn update_task(&mut self, task: &mut TaskBlock);

    /// This is called at the end of the pipeline process of every task processor, to agree on one
    /// task block state and update their internal state
    fn finalize_update(&mut self, task: &TaskBlock);
}

/// A structure respresenting a task that is being process by the task processor, some data may be compleate as the
/// pipeline process go on.
#[derive(Debug, Default)]
struct TaskBlock {
    interrupted_state: TaskProcesserState,
    interrupted_thread: Option<Thread>,
    interrupted_process: Option<Process>,

    new_thread: Vec<Thread>,
    now_dead_thread: Vec<Thread>,

    new_process: Vec<Process>,
    now_dead_process: Vec<Process>,

    scheduled_thread: Option<Thread>,
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
    let mut task_block = TaskBlock {
        interrupted_state: TaskProcesserState::new(&context),
        ..Default::default()
    };

    cpu_local().pipeline().start_processing(&mut task_block);

    Dispatcher::new(task_block)
}

pub fn init(ctx: &mut InitializationContext<End>) {
    ctx.local_initializer(|i| {
        i.register(|builder, ctx, id| {
            builder.control_pipeline(ControlPipeline::new(vec![
                Box::new(ThreadProcessor::new()),
                Box::new(ProcessProcessor::new()),
                Box::new(SchedulerProcessor::new()),
            ]));
        })
    });
}

/// The state of the processor (the actual processor not task processor)
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
