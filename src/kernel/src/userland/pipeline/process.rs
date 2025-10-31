use pager::paging::InactivePageTable;

use crate::{
    memory::stack_allocator::Stack,
    userland::pipeline::{CommonRequestContext, thread::Thread},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    pub fn page_table(&mut self, process: Process) -> &InactivePageTable {
        todo!()
    }

    pub fn alloc_stack(&mut self, process: Process) -> Stack {
        todo!()
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
