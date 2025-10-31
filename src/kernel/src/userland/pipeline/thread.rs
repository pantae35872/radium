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

use core::assert_matches::assert_matches;

use alloc::vec::Vec;

use crate::{
    interrupt::CORE_ID,
    memory::stack_allocator::Stack,
    scheduler::CURRENT_THREAD_ID,
    smp::CoreId,
    userland::pipeline::{
        CommonRequestContext, TaskBlock, TaskProcesserState,
        process::{Process, ProcessPipeline},
    },
};

mod id;

#[derive(Debug)]
pub struct ThreadPipeline {
    pool: Vec<ThreadContext>,
    unused_thread: Vec<usize>,
}

impl ThreadPipeline {
    pub fn new() -> Self {
        Self {
            pool: Vec::new(),
            unused_thread: Vec::new(),
        }
    }

    /// Sync and identify, the thread interrupted with the information from [CommonRequestContext].
    pub fn sync_and_identify(&mut self, context: &CommonRequestContext<'_>) -> Thread {
        let thread = Thread::capture();
        assert_eq!(
            self.thread_context(thread).state,
            ThreadState::Active,
            "Captured thread isn't active"
        );
        self.thread_context_mut(thread).processor_state = TaskProcesserState::new(context);
        thread
    }

    pub fn task_processor_state(&self, thread: Thread) -> &TaskProcesserState {
        &self.thread_context(thread).processor_state
    }

    pub fn free(&mut self, thread: Thread) {
        assert!(
            thread.local_id.core == *CORE_ID,
            "Thread has been migrated without changing the local id"
        );
        let id = thread.local_id.thread;
        id::free_thread(thread);
        self.pool[id].state = ThreadState::Inactive;
        self.unused_thread.push(id);
    }

    /// Allocate a new thread, with the provided parent_process, and a start function
    pub fn alloc<F>(
        &mut self,
        process: &mut ProcessPipeline,
        parent_process: Process,
        start: F,
    ) -> TaskBlock
    where
        F: FnOnce() + Send + 'static,
    {
        if let Some(unused) = self.unused_thread.pop() {
            let thread_ctx = &mut self.pool[unused];
            assert_matches!(
                thread_ctx.state,
                ThreadState::Inactive | ThreadState::Migrated,
                "There shouldn't be an alive thread in the unused thread pool"
            );

            match (
                thread_ctx.state,
                thread_ctx.parent_process == parent_process,
            ) {
                (ThreadState::Migrated, ..) | (ThreadState::Inactive, false) => {
                    *thread_ctx =
                        ThreadContext::new(process.alloc_stack(parent_process), parent_process);
                }
                (ThreadState::Inactive, true) => {
                    thread_ctx.processor_state = TaskProcesserState::default();
                }
                (ThreadState::Active, ..) => {
                    panic!("There shouldn't be an alive thread in the unused thread pool")
                }
            }

            thread_ctx.state = ThreadState::Active;

            return TaskBlock {
                thread: id::alloc_thread(LocalThreadId::new(unused)),
                process: parent_process,
            };
        }

        let new_context = ThreadContext::new(process.alloc_stack(parent_process), parent_process);
        let id = self.pool.len();
        self.pool.push(new_context);
        TaskBlock {
            thread: id::alloc_thread(LocalThreadId::new(id)),
            process: parent_process,
        }
    }

    fn thread_context(&self, thread: Thread) -> &ThreadContext {
        &self.pool[thread.local_id.thread]
    }

    fn thread_context_mut(&mut self, thread: Thread) -> &mut ThreadContext {
        &mut self.pool[thread.local_id.thread]
    }
}

impl Default for ThreadPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
struct ThreadContext {
    state: ThreadState,
    processor_state: TaskProcesserState,
    parent_process: Process,
    stack: Stack,
}

impl ThreadContext {
    fn new(stack: Stack, parent: Process) -> Self {
        Self {
            state: ThreadState::Active,
            processor_state: TaskProcesserState::empty(),
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
#[derive(Debug, Clone, Copy)]
pub struct Thread {
    global_id: usize,
    local_id: LocalThreadId,
}

impl Thread {
    fn capture() -> Self {
        let global_id = *CURRENT_THREAD_ID;
        let local_id = id::translate_to_local(global_id);
        Self {
            global_id,
            local_id,
        }
    }

    pub fn id(&self) -> usize {
        self.global_id
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
        LocalThreadId {
            core: *CORE_ID,
            thread: thread_id,
        }
    }
}
