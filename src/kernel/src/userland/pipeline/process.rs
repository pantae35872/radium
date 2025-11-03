use alloc::{sync::Arc, vec::Vec};
use kernel_proc::IPPacket;
use pager::paging::{InactivePageTable, create_mappings};
use spin::Mutex;

use crate::{
    memory::stack_allocator::Stack,
    smp::CTX,
    userland::pipeline::{CommonRequestContext, thread::Thread},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Process {
    id: usize,
}

#[derive(Default)]
pub struct ProcessPipeline {
    shared_data: Vec<Arc<ProcessShared>>,
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
        CTX.lock().with_inactive(
            todo!("Get the page table for each process"),
            |_mapper, _allocator| {},
        );
        todo!()
    }

    pub fn alloc(&mut self) -> Process {
        todo!()
    }
}

struct ProcessShared {
    stacks: Mutex<Vec<Stack>>,
    page_table: Mutex<InactivePageTable>,
}

impl ProcessShared {
    pub fn new() -> Self {
        todo!()
    }
}

#[derive(Clone, IPPacket)]
struct ExpandSharedPacket {
    expanded: Arc<ProcessShared>,
}
