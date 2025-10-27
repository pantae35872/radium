use crate::userland::pipeline::{CommonRequestContext, thread::Thread};

#[derive(Debug, Clone, Copy)]
pub struct Process {
    id: usize,
}

#[derive(Debug)]
pub struct ProcessPipeline {}

impl ProcessPipeline {
    pub fn new() -> Self {
        Self {}
    }

    pub fn sync_and_identify(
        &mut self,
        context: &CommonRequestContext<'_>,
        thread: &Thread,
    ) -> Process {
        todo!("Identify the process from the thread")
    }

    pub fn alloc(&mut self) -> Process {
        todo!()
    }
}

impl Default for ProcessPipeline {
    fn default() -> Self {
        Self::new()
    }
}
