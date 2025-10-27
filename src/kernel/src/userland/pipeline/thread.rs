use alloc::vec::Vec;

use crate::{
    memory::stack_allocator::Stack,
    smp::{CoreId, cpu_local},
    userland::pipeline::{
        CommonRequestContext, TaskBlock, TaskProcesserState,
        process::{Process, ProcessPipeline},
        thread::id::translate_to_local,
    },
};

mod id;

#[derive(Debug)]
pub struct ThreadPipeline {
    pool: Vec<ThreadContext>,
    dead_thread: Vec<usize>,
    invalid_thread: Vec<usize>,
}

impl ThreadPipeline {
    pub fn new() -> Self {
        Self {
            pool: Vec::new(),
            dead_thread: Vec::new(),
            invalid_thread: Vec::new(),
        }
    }

    pub fn sync_and_identify(&mut self, context: &CommonRequestContext<'_>) -> Thread {
        let thread = Thread::capture();
        assert!(self.thread_context(thread).alive);
        self.thread_context_mut(thread).state = TaskProcesserState::new(context);
        thread
    }

    pub fn task_processor_state(&self, task: TaskBlock) -> &TaskProcesserState {
        &self.thread_context(task.thread).state
    }

    pub fn alloc<F>(
        &mut self,
        process: &mut ProcessPipeline,
        parent_process: Process,
        start: F,
    ) -> TaskBlock
    where
        F: FnOnce() + Send + 'static,
    {
        todo!()
        //if let Some(dead) = self.dead_thread.pop() {
        //    let thread_ctx = &mut self.pool[dead];
        //    assert!(
        //        !thread_ctx.alive,
        //        "Invalid state, there's alive thread in the dead thread pool"
        //    );
        //    thread_ctx.alive = true;
        //    return Ok(Thread::new(
        //        f,
        //        LocalThreadId::create_local(dead as u32),
        //        thread_ctx,
        //    ));
        //}

        //if let Some(id) = self.invalid_thread.pop() {
        //    let new_context = ThreadPool::alloc_context(&mut cpu_local().ctx().lock())?;
        //    let thread = Thread::new(f, LocalThreadId::create_local(id as u32), &new_context);
        //    self.pool[id] = new_context;
        //    return Ok(thread);
        //}

        //let new_context = ThreadPool::alloc_context(&mut cpu_local().ctx().lock())?;
        //let id = self.pool.len();
        //let thread = Thread::new(f, LocalThreadId::create_local(id as u32), &new_context);
        //self.pool.push(new_context);
        //Ok(thread)
    }

    fn thread_context(&self, id: Thread) -> &ThreadContext {
        &self.pool[id.local_id.thread as usize]
    }

    fn thread_context_mut(&mut self, id: Thread) -> &mut ThreadContext {
        &mut self.pool[id.local_id.thread as usize]
    }
}

impl Default for ThreadPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
struct ThreadContext {
    alive: bool,
    state: TaskProcesserState,
    stack: Stack,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Thread {
    global_id: usize,
    local_id: LocalThreadId,
}

impl Thread {
    fn capture() -> Self {
        let global_id = cpu_local().current_thread_id();
        let local_id = translate_to_local(global_id);
        Self {
            global_id,
            local_id,
        }
    }

    pub fn id(&self) -> usize {
        self.global_id
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LocalThreadId {
    core: CoreId,
    thread: u32,
}
