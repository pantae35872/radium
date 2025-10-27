use alloc::vec::Vec;
use spin::RwLock;

use crate::userland::pipeline::thread::LocalThreadId;

static GLOBAL_THREAD_ID_MAP: RwLock<GlobalThreadIdPool> = RwLock::new(GlobalThreadIdPool::new());

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

pub fn translate_to_local(global_id: usize) -> LocalThreadId {
    GLOBAL_THREAD_ID_MAP.read().translate(global_id)
}
