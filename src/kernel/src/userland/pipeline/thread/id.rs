use core::{
    num::NonZeroUsize,
    sync::atomic::{AtomicUsize, Ordering},
};

use alloc::vec::Vec;
use spin::RwLock;

use crate::userland::pipeline::thread::{LocalThreadId, Thread};

static GLOBAL_THREAD_ID_MAP: RwLock<GlobalThreadIdPool> = RwLock::new(GlobalThreadIdPool::new());
static SIG: AtomicUsize = AtomicUsize::new(1);

fn sig() -> usize {
    SIG.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug)]
struct GlobalThreadIdPool {
    pool: Vec<GlobalThreadIdData>,
    free_id: Vec<usize>,
}

impl GlobalThreadIdPool {
    const fn new() -> Self {
        Self {
            pool: Vec::new(),
            free_id: Vec::new(),
        }
    }

    #[inline]
    fn translate(&self, global_id: NonZeroUsize) -> LocalThreadId {
        self.pool[global_id.get() - 1].local_id
    }

    fn migrate(&mut self, global_id: NonZeroUsize, new_local_id: LocalThreadId) {
        self.pool[global_id.get() - 1].local_id = new_local_id;
    }

    fn invalidate(&mut self, global_id: NonZeroUsize) {
        self.pool[global_id.get() - 1].signature = sig();
    }

    fn signature(&self, global_id: NonZeroUsize) -> usize {
        self.pool[global_id.get() - 1].signature
    }

    fn alloc(&mut self, local_id: LocalThreadId) -> NonZeroUsize {
        if let Some(free) = self.free_id.pop() {
            let id = &mut self.pool[free];

            id.local_id = local_id;
            id.signature = sig();

            return NonZeroUsize::new(free + 1).unwrap();
        }

        let id = self.pool.len();
        self.pool.push(GlobalThreadIdData {
            local_id,
            signature: sig(),
        });
        NonZeroUsize::new(id + 1).unwrap()
    }

    fn free(&mut self, global_id: NonZeroUsize) {
        self.free_id.push(global_id.get() - 1);

        // Signature 0 is always invalid
        self.pool[global_id.get() - 1].signature = 0;
    }
}

#[derive(Debug)]
struct GlobalThreadIdData {
    local_id: LocalThreadId,
    signature: usize,
}

pub(super) fn translate_to_local(global_id: NonZeroUsize) -> LocalThreadId {
    GLOBAL_THREAD_ID_MAP.read().translate(global_id)
}

pub(super) fn sigature(global_id: NonZeroUsize) -> usize {
    GLOBAL_THREAD_ID_MAP.read().signature(global_id)
}

pub(super) fn alloc_thread(local_id: LocalThreadId) -> Thread {
    let mut map = GLOBAL_THREAD_ID_MAP.write();
    let global_id = map.alloc(local_id);
    Thread {
        global_id,
        signature: map.signature(global_id),
    }
}

pub(super) fn free_thread(thread: Thread) {
    GLOBAL_THREAD_ID_MAP.write().free(thread.global_id);
}

pub(super) fn invalidate(thread: Thread) {
    GLOBAL_THREAD_ID_MAP.write().invalidate(thread.global_id);
}

pub(super) fn migrate_thread(global_id: NonZeroUsize, local_id: LocalThreadId) {
    GLOBAL_THREAD_ID_MAP.write().migrate(global_id, local_id);
}
