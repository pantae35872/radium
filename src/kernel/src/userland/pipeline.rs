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

use core::cell::RefCell;

use alloc::vec::Vec;
use kernel_proc::{def_local, local_builder};
use pager::{address::VirtAddr, registers::RFlags};
use santa::Elf;
use sentinel::log;
use smart_default::SmartDefault;

use crate::{
    initialization_context::{InitializationContext, Stage4},
    interrupt::{self, InterruptIndex},
    userland::{
        pipeline::{
            dispatch::Dispatcher,
            process::{Process, ProcessPipeline},
            scheduler::SchedulerPipeline,
            thread::{Thread, ThreadPipeline},
        },
        syscall::SyscallId,
        PACKED_DATA,
    },
};

pub mod dispatch;
mod process;
mod scheduler;
mod thread;

pub fn init(ctx: &mut InitializationContext<Stage4>) {
    ctx.local_initializer(|i| {
        i.register(|builder, _ctx, _id| {
            local_builder!(
                builder,
                PIPELINE(ControlPipeline::new().into()),
                CURRENT_THREAD_ID(0.into())
            );
        });
    });
}

def_local!(static PIPELINE: RefCell<crate::userland::pipeline::ControlPipeline>);
def_local!(pub static CURRENT_THREAD_ID: RefCell<usize>);

pub fn timer_count() -> usize {
    interrupt::without_interrupts(|| PIPELINE.borrow().timer_count())
}

pub fn spawn_init() {
    interrupt::without_interrupts(|| {
        PIPELINE.borrow_mut().spawn_init();
    });
}

pub fn start_scheduling() {
    interrupt::without_interrupts(|| {
        PIPELINE.borrow_mut().should_schedule = true;
    });
}

/// A cpu local structure that contains all the specific request independent pipelines (the state is shared
/// across request), use [`ControlPipeline::create_context`] to create the specific request
/// context.
pub struct ControlPipeline {
    thread: ThreadPipeline,
    process: ProcessPipeline,
    scheduler: SchedulerPipeline,

    events: Option<Event>,
    should_schedule: bool,
}

#[derive(Debug, Default)]
struct Event {
    hw_interrupts: Vec<fn(&mut ControlPipeline, InterruptIndex)>,
    ipp_handlers: Vec<fn(&mut ControlPipeline)>,
    finalize: Vec<fn(&mut ControlPipeline, &mut PipelineContext)>,
    begin: Vec<fn(&mut ControlPipeline, &mut PipelineContext, &CommonRequestContext)>,
}

impl Event {
    fn begin(
        &mut self,
        handler: fn(&mut ControlPipeline, &mut PipelineContext, &CommonRequestContext),
    ) {
        self.begin.push(handler);
    }

    fn finalize(&mut self, handler: fn(&mut ControlPipeline, &mut PipelineContext)) {
        self.finalize.push(handler);
    }

    fn hw_interrupts(&mut self, handler: fn(&mut ControlPipeline, InterruptIndex)) {
        self.hw_interrupts.push(handler);
    }

    fn ipp_handler(&mut self, handler: fn(&mut ControlPipeline)) {
        self.ipp_handlers.push(handler);
    }
}

#[derive(Debug, Default)]
pub struct PipelineContext {
    pub interrupted_thread: Option<Thread>,
    pub interrupted_process: Option<Process>,
    pub interrupted_task: Option<TaskBlock>,
    pub added_tasks: Vec<TaskBlock>,
    pub should_schedule: bool,
    pub scheduled_task: Option<TaskBlock>,
}

impl ControlPipeline {
    fn new() -> Self {
        let mut events = Event::default();

        let thread = ThreadPipeline::new(&mut events);
        let process = ProcessPipeline::new(&mut events);

        events.begin(|_, ctx, _| {
            ctx.interrupted_task = ctx.interrupted_thread.and_then(|thread| {
                Some(TaskBlock {
                    thread,
                    process: ctx.interrupted_process?,
                })
            });
        });
        let scheduler = SchedulerPipeline::new(&mut events);

        Self {
            thread,
            process,
            scheduler,
            events: Some(events),
            should_schedule: false,
        }
    }

    pub fn timer_count(&self) -> usize {
        self.scheduler.timer_count()
    }

    fn spawn_init(&mut self) {
        let packed = PACKED_DATA.get().unwrap();
        let init_program = packed
            .iter()
            .find(|e| e.name == "init")
            .expect("Can't find init!");

        let init_program = Elf::new(init_program.data).expect("Init is not a valid elf");
        let process = self.alloc_process();
        let entry = self.process.mem_access(
            // SAFETY: The mem access uphold the contract
            |_process, mapper, allocator| unsafe {
                init_program.load_user(&mut mapper.mapper_with_allocator(allocator))
            },
            process,
        );

        log!(Debug, "Init program entry at 0x{entry:x}");

        self.scheduler
            .add_task(self.thread.alloc(&mut self.process, process, entry));
    }

    pub fn sleep_task(&mut self, task: TaskBlock, millis: usize) {
        self.scheduler.sleep_task(task, millis);
    }

    pub fn free_thread(&mut self, thread: Thread) {
        self.thread.free(thread);
        self.process.free_thread(thread);
    }

    pub fn free_process(&mut self, process: Process) {
        self.process.free(&mut self.thread, process);
    }

    pub fn alloc_thread(
        &mut self,
        context: &mut PipelineContext,
        parent_process: Process,
        start: VirtAddr,
    ) -> TaskBlock {
        let task = self.thread.alloc(&mut self.process, parent_process, start);
        context.added_tasks.push(task);
        task
    }

    pub fn alloc_process(&mut self) -> Process {
        self.process.alloc()
    }

    fn create_context(&mut self, context: &CommonRequestContext<'_>) -> PipelineContext {
        let mut ctx = PipelineContext {
            should_schedule: self.should_schedule,
            ..Default::default()
        };

        if let Some(event) = self.events.take() {
            for handler in event.begin.iter() {
                handler(self, &mut ctx, context)
            }
            self.events = Some(event);
        }

        ctx
    }

    fn hardware_interrupt(&mut self, index: InterruptIndex) {
        if let Some(event) = self.events.take() {
            for handler in event.hw_interrupts.iter() {
                handler(self, index)
            }
            self.events = Some(event);
        }
    }

    fn finalize(&mut self, context: &mut PipelineContext) {
        if let Some(event) = self.events.take() {
            for handler in event.finalize.iter() {
                handler(self, context)
            }
            self.events = Some(event);
        }
    }

    fn handle_ipp(&mut self, context: &mut PipelineContext) {
        if let Some(event) = self.events.take() {
            for handler in event.ipp_handlers.iter() {
                handler(self)
            }
            self.events = Some(event);
        }

        context.should_schedule = false;
    }

    fn schedule(&mut self, context: &mut PipelineContext) {
        if !context.should_schedule {
            return;
        }

        self.scheduler.schedule(&mut self.thread, context);
    }
}

/// A lightweight struct to store just enough data to know which process or thread, we're talking
/// about (an indirect reference)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskBlock {
    pub thread: Thread,
    pub process: Process,
}

impl TaskBlock {
    /// When the process id or thread id is reused, this will return false
    pub fn valid(&self) -> bool {
        self.thread.valid() && self.process.valid()
    }
}

/// Handle the request with the provided [`CommonRequestContext`], returning a dispatcher
/// [`Dispatcher`] that must be used to operate the right following actions.
pub fn handle_request<'b>(
    rq_context: CommonRequestContext<'b>,
    dispatch: impl for<'a> FnOnce(CommonRequestContext<'b>, Dispatcher<'a>),
) {
    let mut pipeline = PIPELINE.borrow_mut();
    let mut context = pipeline.create_context(&rq_context);
    match rq_context.referer {
        RequestReferer::SyscallRequest(id) => {
            super::syscall::syscall_handle(&rq_context, &mut pipeline, &mut context, id)
        }
        RequestReferer::HardwareInterrupt(InterruptIndex::CheckIPP) => {
            pipeline.handle_ipp(&mut context);
        }
        RequestReferer::HardwareInterrupt(i) => {
            pipeline.hardware_interrupt(i);
        }
    }
    pipeline.schedule(&mut context);
    pipeline.finalize(&mut context);
    dispatch(rq_context, Dispatcher::new(context, &pipeline.thread))
}

#[derive(Debug, SmartDefault)]
pub struct CommonRequestStackFrame {
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

    #[default(VirtAddr::null())]
    pub instruction_pointer: VirtAddr,
    #[default(RFlags::ID | RFlags::AlignmentCheck | RFlags::InterruptEnable)]
    pub cpu_flags: RFlags,
    #[default(VirtAddr::null())]
    pub stack_pointer: VirtAddr,
}

impl CommonRequestStackFrame {
    fn new() -> Self {
        Self::default()
    }

    pub fn replace_with(&mut self, task: &TaskProcesserState) {
        self.r15 = task.r15;
        self.r14 = task.r14;
        self.r13 = task.r13;
        self.r12 = task.r12;
        self.r11 = task.r11;
        self.r10 = task.r10;
        self.r9 = task.r9;
        self.r8 = task.r8;
        self.rsi = task.rsi;
        self.rdi = task.rdi;
        self.rbp = task.rbp;
        self.rdx = task.rdx;
        self.rcx = task.rcx;
        self.rbx = task.rbx;
        self.rax = task.rax;
        self.instruction_pointer = task.instruction_pointer;
        self.cpu_flags = task.cpu_flags;
        self.stack_pointer = task.stack_pointer;
    }
}

/// A structure describing the context of the requester.
pub struct CommonRequestContext<'a> {
    pub stack_frame: &'a mut CommonRequestStackFrame,
    pub referer: RequestReferer,
}

impl<'a> CommonRequestContext<'a> {
    pub fn new(stack_frame: &'a mut CommonRequestStackFrame, referer: RequestReferer) -> Self {
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

#[derive(Debug, Clone, SmartDefault, PartialEq, Eq)]
pub struct TaskProcesserState {
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
    #[default(VirtAddr::null())]
    pub instruction_pointer: VirtAddr,
    #[default(RFlags::ID | RFlags::AlignmentCheck | RFlags::InterruptEnable)]
    pub cpu_flags: RFlags,
    #[default(VirtAddr::null())]
    pub stack_pointer: VirtAddr,

    pub extended_state: ExtendedState,
}

impl<'a> From<&CommonRequestContext<'a>> for TaskProcesserState {
    fn from(context: &CommonRequestContext<'a>) -> Self {
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
            cpu_flags: context.stack_frame.cpu_flags,
            stack_pointer: context.stack_frame.stack_pointer,
            instruction_pointer: context.stack_frame.instruction_pointer,
            extended_state: ExtendedState,
        }
    }
}

// TODO: Implement Extened States (XSAVE, https://www.felixcloutier.com/x86/xsave)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtendedState;
