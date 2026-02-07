//! A pipeline managing threads on a local cpu core.
//!
//! This module ([`ThreadPipeline`]) provides an abstraction over thread resources management, **Use** [`ThreadPipeline::alloc`] **to
//! allocate a new thread**, and [`ThreadPipeline::free`] **to free up a thread**. Some thread resources (e.g. stack) **might
//! be reused** when seem appropriate (e.g. the old thread have the same parent process as the new one,
//! the old thread hasn't been migrated to a different core, ...).
//!
//! This pipeline interfaces mostly operate with [`Thread`] structure, It can be thought of as a reference
//! to the thread resources, This is done because directly giving a reference to the thread resources
//! can causes borrow checker problems.

use core::{assert_matches, mem::zeroed, num::NonZeroUsize};

use alloc::vec::Vec;
use kernel_proc::IPPacket;
use pager::address::VirtAddr;

use crate::{
    interrupt::CORE_ID,
    memory::stack_allocator::Stack,
    smp::CoreId,
    userland::pipeline::{
        CURRENT_THREAD_ID, CommonRequestContext, Event, PipelineContext, TaskBlock, TaskProcesserState,
        process::{Process, ProcessPipeline},
        scheduler::SchedulerPipeline,
    },
};

mod id;

#[derive(Debug)]
pub struct ThreadPipeline {
    pool: Vec<ThreadContext>,
    unused_thread: Vec<usize>,
}

impl ThreadPipeline {
    pub(super) fn new(event: &mut Event) -> Self {
        event.begin(|c, pipeline_context, request_context| {
            pipeline_context.interrupted_thread = c.thread.begin(request_context);
        });

        event.finalize(|c, s| c.thread.finalize(s));

        event.ipp_handler(|c, cx| c.thread.handle_ipp(cx, &mut c.scheduler));

        Self { pool: Vec::new(), unused_thread: Vec::new() }
    }

    fn begin(&mut self, context: &CommonRequestContext<'_>) -> Option<Thread> {
        let thread = Thread::capture()?;
        assert_eq!(self.thread_context(thread).state, ThreadState::Active, "Captured thread isn't active");
        self.thread_context_mut(thread).processor_state = TaskProcesserState::from(context);
        Some(thread)
    }

    fn finalize(&mut self, ctx: &mut PipelineContext) {
        if !ctx.should_schedule {
            return;
        }

        if let Some(task) = ctx.scheduled_task {
            *CURRENT_THREAD_ID.borrow_mut() = task.thread.global_id.get();
        } else {
            *CURRENT_THREAD_ID.borrow_mut() = 0;
        }
    }

    pub fn task_processor_state(&self, thread: Thread) -> &TaskProcesserState {
        &self.thread_context(thread).processor_state
    }

    fn handle_ipp(&mut self, _context: &mut PipelineContext, scheduler: &mut SchedulerPipeline) {
        ThreadMigratePacket::handle(|ThreadMigratePacket { context, process, global_id }| {
            if let Some(unused) =
                self.unused_thread.iter().find(|e| matches!(self.pool[**e].state, ThreadState::Migrated))
            {
                let thread_ctx = &mut self.pool[*unused];
                *thread_ctx = context;

                id::migrate_thread(global_id, LocalThreadId::new(*unused));
            } else {
                let id = self.pool.len();
                self.pool.push(context);

                id::migrate_thread(global_id, LocalThreadId::new(id));
            }
            let thread = Thread { global_id, signature: id::sigature(global_id) };

            scheduler.add_task(TaskBlock { process, thread });
        });
    }

    pub fn migrate(&mut self, destination: CoreId, TaskBlock { thread, process }: TaskBlock) {
        let id = thread.local_id().thread;

        let context = core::mem::replace(
            self.thread_context_mut(thread),
            ThreadContext {
                state: ThreadState::Migrated,
                // SAFETY: This is fine since the new ThreadState is set to migrated which will be
                // replaced with the new context when this is reused
                ..unsafe { zeroed() }
            },
        );
        ThreadMigratePacket { context, global_id: thread.global_id, process }.send(destination, false);

        id::invalidate(thread);
        self.unused_thread.push(id);
    }

    pub fn free(&mut self, thread: Thread) {
        assert!(thread.local_id().core == *CORE_ID, "Thread has been migrated without changing the local id");
        let id = thread.local_id().thread;
        id::free_thread(thread);
        self.pool[id].state = ThreadState::Inactive;
        self.unused_thread.push(id);
    }

    /// Allocate a new thread, with the provided parent_process, and a start address
    pub fn alloc(&mut self, process: &mut ProcessPipeline, parent_process: Process, start: VirtAddr) -> TaskBlock {
        if let Some(unused) = self.unused_thread.pop() {
            let thread_ctx = &mut self.pool[unused];
            assert_matches!(
                thread_ctx.state,
                ThreadState::Inactive | ThreadState::Migrated,
                "There shouldn't be an alive thread in the unused thread pool"
            );

            match (thread_ctx.state, thread_ctx.parent_process == parent_process) {
                (ThreadState::Migrated, ..) | (ThreadState::Inactive, false) => {
                    // FIXME: This leaks the stack of the previous parent process
                    *thread_ctx = ThreadContext::new(process.alloc_stack(parent_process), parent_process, start);
                }
                (ThreadState::Inactive, true) => {
                    // TODO: Zero out the stack if possible
                    thread_ctx.processor_state = TaskProcesserState {
                        instruction_pointer: start,
                        stack_pointer: thread_ctx.stack.top(),
                        ..Default::default()
                    };
                }
                (ThreadState::Active, ..) => {
                    panic!("There shouldn't be an alive thread in the unused thread pool")
                }
            }

            thread_ctx.state = ThreadState::Active;
            let thread = id::alloc_thread(LocalThreadId::new(unused));
            process.alloc_thread(parent_process, thread);

            return TaskBlock { thread, process: parent_process };
        }

        let new_context = ThreadContext::new(process.alloc_stack(parent_process), parent_process, start);
        let id = self.pool.len();
        self.pool.push(new_context);

        let thread = id::alloc_thread(LocalThreadId::new(id));
        process.alloc_thread(parent_process, thread);

        TaskBlock { thread, process: parent_process }
    }

    fn thread_context(&self, thread: Thread) -> &ThreadContext {
        let context = &self.pool[thread.local_id().thread];
        assert_ne!(context.state, ThreadState::Migrated, "trying to access a migrated thread");
        context
    }

    fn thread_context_mut(&mut self, thread: Thread) -> &mut ThreadContext {
        &mut self.pool[thread.local_id().thread]
    }
}

#[derive(Debug, IPPacket)]
struct ThreadMigratePacket {
    context: ThreadContext,
    global_id: NonZeroUsize,
    process: Process,
}

#[derive(Debug)]
struct ThreadContext {
    state: ThreadState,
    processor_state: TaskProcesserState,
    parent_process: Process,
    stack: Stack,
}

impl ThreadContext {
    fn new(stack: Stack, parent: Process, start: VirtAddr) -> Self {
        Self {
            state: ThreadState::Active,
            processor_state: TaskProcesserState {
                instruction_pointer: start,
                stack_pointer: stack.top(),
                ..Default::default()
            },
            parent_process: parent,
            stack,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThreadState {
    /// Thread is active (alive).
    Active,
    /// Thread is inactive (dead), some data can be **reused, ONLY IF** the parent process is the same as
    /// the new one.
    Inactive,
    /// Thread is migrated to a different core, the context must be recreated, **DO NOT REUSE** any of
    /// the data
    Migrated,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Thread {
    global_id: NonZeroUsize,

    /// Used to indicate if [`Thread::global_id`] has been reused, or freed
    signature: usize,
}

impl Thread {
    pub fn valid(&self) -> bool {
        self.signature == id::sigature(self.global_id)
    }

    pub fn id(&self) -> NonZeroUsize {
        self.global_id
    }

    fn capture() -> Option<Self> {
        if *CURRENT_THREAD_ID.borrow() == 0 {
            return None;
        }

        let current = NonZeroUsize::new(*CURRENT_THREAD_ID.borrow()).unwrap();

        Some(Self { global_id: current, signature: id::sigature(current) })
    }

    fn local_id(&self) -> LocalThreadId {
        id::translate_to_local(self.global_id)
    }
}

/// A thread id but in a specific cpu
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct LocalThreadId {
    core: CoreId,
    thread: usize,
}

impl LocalThreadId {
    pub fn new(thread_id: usize) -> LocalThreadId {
        LocalThreadId { core: *CORE_ID, thread: thread_id }
    }
}
