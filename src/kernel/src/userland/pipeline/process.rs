use alloc::{sync::Arc, vec::Vec};
use kernel_proc::IPPacket;
use pager::paging::InactivePageTable;
use spin::Mutex;

use crate::{
    memory::stack_allocator::Stack,
    userland::pipeline::{CommonRequestContext, thread::Thread},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Process {
    id: usize,
}

#[derive(Debug, Default)]
pub struct ProcessPipeline {
    shared_data: Vec<Arc<ProcessShared>>,
}

#[derive(Debug, Default)]
struct ProcessShared {
    stacks: Mutex<Vec<Stack>>,
}

impl ProcessPipeline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sync_and_identify(
        &mut self,
        context: &CommonRequestContext<'_>,
        thread: &Thread,
    ) -> Process {
        todo!("Identify the process from the thread")
    }

    pub fn check_ipp(&mut self) {
        ExpandSharedPacket::handle(|packet| {
            self.shared_data.push(packet.expanded);
        });
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

#[derive(Clone, IPPacket)]
struct ExpandSharedPacket {
    expanded: Arc<ProcessShared>,
}
