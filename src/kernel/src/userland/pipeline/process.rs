use alloc::{sync::Arc, vec::Vec};
use kernel_proc::IPPacket;
use pager::{
    address::Page,
    paging::{
        InactivePageCopyOption, InactivePageTable,
        table::{RecurseLevel4, RecurseLevel4LowerHalf},
    },
};
use spin::Mutex;

use crate::{
    memory::{
        copy_mappings, create_mappings_lower,
        stack_allocator::{Stack, StackAllocator},
    },
    userland::{
        self,
        pipeline::{CommonRequestContext, thread::Thread},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Process {
    id: usize,
}

#[derive(Default)]
pub struct ProcessPipeline {
    shared_data: Vec<Arc<ProcessShared>>,
    page_tables: Vec<Option<InactivePageTable<RecurseLevel4LowerHalf>>>,
    free_data: Vec<usize>,
}

impl ProcessPipeline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sync_and_identify(
        &mut self,
        _context: &CommonRequestContext<'_>,
        _thread: &Thread,
    ) -> Process {
        todo!("Identify the process from the thread")
    }

    pub fn check_ipp(&mut self) {
        ExpandSharedPacket::handle(|packet| {
            self.shared_data.push(packet.expanded);

            self.page_tables.push(Some(unsafe {
                copy_mappings(InactivePageCopyOption::lower_half(), &packet.table_template)
            }));
        });
    }

    pub fn page_table(&mut self, _process: Process) -> &InactivePageTable<RecurseLevel4> {
        todo!()
    }

    pub fn alloc_stack(&mut self, _process: Process) -> Stack {
        todo!()
    }

    pub fn alloc(&mut self) -> Process {
        if let Some(free_data) = self.free_data.pop() {
            return Process { id: free_data };
        }
        let id = self.shared_data.len();
        let expanded = Arc::new(ProcessShared::new());
        self.shared_data.push(Arc::clone(&expanded));

        // SAFETY: TODO
        let orignal_table = Arc::new(unsafe {
            create_mappings_lower(
                |mapper, alloc| {
                    mapper.populate_p4_lower_half(alloc);
                },
                InactivePageCopyOption::Empty,
            )
        });
        ExpandSharedPacket {
            expanded,
            table_template: Arc::clone(&orignal_table),
        }
        .broadcast(false);
        // SAFETY: TODO
        self.page_tables.push(Some(unsafe {
            copy_mappings(InactivePageCopyOption::lower_half(), &orignal_table)
        }));

        Process { id }
    }
}

struct ProcessShared {
    stacks: Mutex<StackAllocator>,
}

impl ProcessShared {
    pub fn new() -> Self {
        // SAFETY: The safety section of the create_mappings doesn't apply when the used variant of
        // [`InactivePageCopyOption`] is Empty
        Self {
            stacks: StackAllocator::new(Page::range_inclusive(
                userland::STACK_START.into(),
                (userland::STACK_START + userland::STACK_MAX_SIZE).into(),
            ))
            .into(),
        }
    }
}

#[derive(Clone, IPPacket)]
struct ExpandSharedPacket {
    expanded: Arc<ProcessShared>,
    table_template: Arc<InactivePageTable<RecurseLevel4LowerHalf>>,
}
