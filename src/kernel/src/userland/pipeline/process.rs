use alloc::{sync::Arc, vec::Vec};
use kernel_proc::IPPacket;
use pager::{
    address::Page,
    paging::{
        InactivePageCopyOption, InactivePageTable,
        mapper::{Mapper, MapperWithAllocator},
        table::RecurseLevel4LowerHalf,
    },
};
use spin::Mutex;

use crate::{
    memory::{
        allocator::buddy_allocator::BuddyAllocator,
        copy_mappings, create_mappings_lower, mapper_lower, mapper_lower_with,
        stack_allocator::{Stack, StackAllocator},
        switch_lower_half,
    },
    userland::{
        self,
        pipeline::{CommonRequestContext, Event, PipelineContext, thread::Thread},
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
    pub(super) fn new(events: &mut Event) -> Self {
        events.ipp_handler(|c| c.process.check_ipp());

        Self::default()
    }

    pub fn sync_and_identify(
        &mut self,
        _context: &CommonRequestContext<'_>,
        thread: &Thread,
    ) -> Process {
        // FIXME: probably use a map instead
        for (id, shared) in self.shared_data.iter().enumerate() {
            if shared.threads.lock().iter().any(|t| *t == thread.id()) {
                return Process { id };
            }
        }
        panic!("thread id {} doesn't belong to any processes", thread.id());
    }

    pub fn finalize(&mut self, context: &mut PipelineContext) {
        match (context.interrupted_task, context.scheduled_task) {
            (Some(interrupted), Some(scheduled)) if interrupted == scheduled => {
                self.page_table_swap(interrupted.process, scheduled.process);
            }
            _ => {}
        }
    }

    fn check_ipp(&mut self) {
        ExpandSharedPacket::handle(|packet| {
            self.shared_data.push(packet.expanded);

            self.page_tables.push(Some(unsafe {
                copy_mappings(InactivePageCopyOption::lower_half(), &packet.table_template)
            }));
        });
    }

    pub fn page_table_swap(&mut self, from: Process, with: Process) {
        assert_ne!(from, with);

        let with = self.page_tables[with.id]
            .take()
            .expect("Page table scheduled two times");

        assert!(
            self.page_tables[from.id].is_none(),
            "Page table scheduled two times"
        );
        self.page_tables[from.id] = Some(switch_lower_half(with));
    }

    pub fn mapper<R>(
        &mut self,
        f: impl FnOnce(&mut Self, &mut Mapper<RecurseLevel4LowerHalf>, &mut BuddyAllocator) -> R,
        process: Process,
    ) -> R {
        // If the table is present in the page_tables then it's not active, use that table as a
        // mapper
        // FIXME: RACECONDITION!!!!!!, if the table is active in the other cores, there might be
        // a race condition
        if let Some(mut table) = self.page_tables[process.id].take() {
            // SAFETY: Read the caller of the
            let r = unsafe {
                mapper_lower_with(|mapper, allocator| f(self, mapper, allocator), &mut table)
            };
            self.page_tables[process.id] = Some(table);
            r
        } else {
            // If the table is currently active, use that as a mapper
            mapper_lower(|MapperWithAllocator { mapper, allocator }| f(self, mapper, allocator))
        }
    }

    pub fn alloc_stack(&mut self, process: Process) -> Stack {
        self.mapper(
            |s, mapper, allocator| {
                s.shared_data(process)
                    .stacks
                    .lock()
                    .alloc_stack(mapper, allocator, 16)
            },
            process,
        )
        .expect(
            "Can't allocate new stack for process, uhh deal with this, maybe kill the user process",
        )
    }

    fn shared_data(&mut self, process: Process) -> &ProcessShared {
        &self.shared_data[process.id]
    }

    pub fn alloc_thread(&mut self, parent: Process, thread: Thread) {
        self.shared_data[parent.id].threads.lock().push(thread.id());
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
    threads: Mutex<Vec<usize>>,
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
            threads: Vec::new().into(),
        }
    }
}

#[derive(Clone, IPPacket)]
struct ExpandSharedPacket {
    expanded: Arc<ProcessShared>,
    table_template: Arc<InactivePageTable<RecurseLevel4LowerHalf>>,
}
