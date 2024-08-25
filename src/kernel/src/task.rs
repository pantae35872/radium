use core::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll},
};

use alloc::boxed::Box;

pub mod executor;

pub struct Task {
    id: TaskId,
    typ: AwaitType,
    future: Pin<Box<dyn Future<Output = ()>>>,
}

pub enum AwaitType {
    Waker,
    Poll,
}

impl Task {
    pub fn new(future: impl Future<Output = ()> + 'static, typ: AwaitType) -> Task {
        Task {
            id: TaskId::new(),
            future: Box::pin(future),
            typ,
        }
    }

    fn poll(&mut self, contex: &mut Context) -> Poll<()> {
        return self.future.as_mut().poll(contex);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TaskId(u64);

impl TaskId {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        TaskId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
    }
}
