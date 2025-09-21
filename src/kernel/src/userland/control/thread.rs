use alloc::vec::Vec;
use spin::RwLock;

use crate::{
    memory::stack_allocator::Stack,
    smp::{CoreId, cpu_local},
    userland::control::{TaskProcesserState, TaskProcessor},
};

static GLOBAL_THREAD_ID_MAP: RwLock<GlobalThreadIdPool> = RwLock::new(GlobalThreadIdPool::new());

#[derive(Debug)]
pub struct ThreadProcessor {
    pool: Vec<ThreadContext>,
    dead_thread: Vec<usize>,
    invalid_thread: Vec<usize>,
}

impl ThreadProcessor {
    pub fn new() -> Self {
        Self {
            pool: Vec::new(),
            dead_thread: Vec::new(),
            invalid_thread: Vec::new(),
        }
    }
}

impl Default for ThreadProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskProcessor for ThreadProcessor {
    fn update_task(&mut self, task: &mut super::TaskBlock) {
        let global_id = cpu_local().current_thread_id();
        task.interrupted_thread = Some(Thread {
            global_id,
            local_id: GLOBAL_THREAD_ID_MAP.read().translate(global_id),
        });
    }

    fn finalize_update(&mut self, task: &super::TaskBlock) {}
}

#[derive(Debug)]
struct ThreadContext {
    alive: bool,
    state: TaskProcesserState,
    stack: Stack,
}

#[derive(Debug)]
struct GlobalThreadIdPool {
    pool: Vec<GlobalThreadIdData>,
    free_id: Vec<usize>,
}

impl GlobalThreadIdPool {
    pub const fn new() -> Self {
        Self {
            pool: Vec::new(),
            free_id: Vec::new(),
        }
    }

    #[inline]
    fn translate(&self, global_id: usize) -> LocalThreadId {
        self.pool[global_id].local_id
    }

    fn migrate(&mut self, global_id: usize, new_local_id: LocalThreadId) {
        self.pool[global_id].local_id = new_local_id;
    }

    fn alloc(&mut self, local_id: LocalThreadId) -> usize {
        if let Some(free) = self.free_id.pop() {
            let id = &mut self.pool[free];

            id.local_id = local_id;

            return free;
        }

        let id = self.pool.len();
        self.pool.push(GlobalThreadIdData { local_id });
        id
    }

    fn free(&mut self, global_id: usize) -> LocalThreadId {
        self.free_id.push(global_id);
        self.pool[global_id].local_id
    }
}

#[derive(Debug)]
struct GlobalThreadIdData {
    local_id: LocalThreadId,
}

#[repr(C)]
#[derive(Debug)]
pub struct Thread {
    global_id: usize,
    local_id: LocalThreadId,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LocalThreadId {
    core: CoreId,
    thread: u32,
}
