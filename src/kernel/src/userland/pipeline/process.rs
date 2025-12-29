use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::{sync::Arc, vec::Vec};
use hashbrown::HashSet;
use kernel_proc::IPPacket;
use pager::{
    address::Page,
    paging::{
        InactivePageCopyOption, InactivePageTable,
        mapper::{Mapper, MapperWithAllocator},
        table::RecurseLevel4LowerHalf,
    },
};
use spin::{Mutex, RwLock};

use crate::{
    memory::{
        allocator::buddy_allocator::BuddyAllocator,
        copy_mappings, create_mappings_lower, mapper_lower, mapper_lower_with,
        stack_allocator::{Stack, StackAllocator},
        switch_lower_half,
    },
    userland::{
        self,
        pipeline::{
            Event, PipelineContext, TaskBlock,
            thread::{Thread, ThreadPipeline},
        },
    },
};

#[derive(Default)]
pub struct ProcessPipeline {
    page_tables: Vec<Option<InactivePageTable<RecurseLevel4LowerHalf>>>,
    hlt_page_table: Option<InactivePageTable<RecurseLevel4LowerHalf>>,
    free_data: Vec<usize>,
}

impl ProcessPipeline {
    pub(super) fn new(events: &mut Event) -> Self {
        events.begin(|_c, pipeline_context, _request_context| {
            if let Some(thread) = pipeline_context.interrupted_thread {
                pipeline_context.interrupted_process = Some(find_by_id(&thread))
            }
        });

        events.finalize(|c, s| c.process.finalize(s));

        events.ipp_handler(|c| c.process.check_ipp());

        Self::default()
    }

    pub fn finalize(&mut self, context: &mut PipelineContext) {
        match (context.interrupted_task, context.scheduled_task) {
            (Some(interrupted), Some(scheduled)) if interrupted != scheduled => {
                self.page_table_swap(interrupted.process, scheduled.process);
            }
            (None, Some(TaskBlock { process, .. })) => {
                let with = self.page_tables[process.id]
                    .take()
                    .expect("Some one forgot to put back their page table");

                assert!(
                    self.hlt_page_table.is_none(),
                    "HLT page table didn't get swapped"
                );
                self.hlt_page_table = Some(switch_lower_half(with));
            }
            (Some(TaskBlock { process, .. }), None) => {
                let hlt_table = self
                    .hlt_page_table
                    .take()
                    .expect("HLT Page table stolen or uninitialized");
                self.page_tables[process.id] = Some(switch_lower_half(hlt_table));
            }
            _ => {}
        }
    }

    fn check_ipp(&mut self) {
        ExpandSharedPacket::handle(|packet| {
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

    pub fn mem_access<R>(
        &mut self,
        f: impl FnOnce(&mut Self, &mut Mapper<RecurseLevel4LowerHalf>, &mut BuddyAllocator) -> R,
        process: Process,
    ) -> R {
        if let Some(table) = self.page_tables[process.id].take() {
            let old = switch_lower_half(table);

            let r = mapper_lower(|MapperWithAllocator { mapper, allocator }| {
                f(self, mapper, allocator)
            });

            let table = switch_lower_half(old);
            self.page_tables[process.id] = Some(table);
            r
        } else {
            mapper_lower(|MapperWithAllocator { mapper, allocator }| f(self, mapper, allocator))
        }
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
            |_s, mapper, allocator| {
                shared(&process)
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

    pub fn alloc_thread(&mut self, parent: Process, thread: Thread) {
        shared(&parent).threads.lock().insert(thread);
    }

    pub fn free_thread(&mut self, thread: Thread) {
        shared(&find_by_id(&thread)).threads.lock().remove(&thread);
    }

    pub fn free(&mut self, thread_pipeline: &mut ThreadPipeline, process: Process) {
        let shared = shared(&process);
        for thread in shared.threads.lock().iter() {
            assert!(
                thread.valid(),
                "Process pipeline didn't get notified when some thread got freed",
            );
            thread_pipeline.free(*thread);
        }

        shared.threads.lock().clear();
        free(process);
    }

    pub fn alloc(&mut self) -> Process {
        let process = alloc_shared();
        if self.page_tables.get(process.id).is_some() {
            // TODO: Clean up the page tables
            self.mapper(|_s, _mapper, _allocator| {}, process);
        } else {
            let orignal_table = Arc::new(unsafe {
                create_mappings_lower(
                    |mapper, alloc| {
                        mapper.populate_p4_lower_half(alloc);
                    },
                    InactivePageCopyOption::Empty,
                )
            });

            ExpandSharedPacket {
                table_template: Arc::clone(&orignal_table),
            }
            .broadcast(false);

            self.page_tables.push(Some(unsafe {
                copy_mappings(InactivePageCopyOption::lower_half(), &orignal_table)
            }));
        }

        process
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Process {
    id: usize,

    /// Used to indicate if [`Process::id`] has been reused, or freed
    signature: usize,
}

impl Process {
    pub fn valid(&self) -> bool {
        self.signature == sigature(self)
    }
}

fn free(process: Process) {
    GLOBAL_PROCESS_DATA.write().free(process);
}

fn sigature(process: &Process) -> usize {
    *shared(process).signature.lock()
}

fn find_by_id(thread: &Thread) -> Process {
    GLOBAL_PROCESS_DATA.read().find_by_id(thread)
}

fn alloc_shared() -> Process {
    GLOBAL_PROCESS_DATA.write().alloc()
}

fn shared(process: &Process) -> Arc<ProcessShared> {
    GLOBAL_PROCESS_DATA.read().shared(process)
}

static GLOBAL_PROCESS_DATA: RwLock<GlobalProcessDataPool> =
    RwLock::new(GlobalProcessDataPool::new());

#[derive(Default)]
struct GlobalProcessDataPool {
    pool: Vec<Arc<ProcessShared>>,
    free_id: Vec<usize>,
}

impl GlobalProcessDataPool {
    const fn new() -> Self {
        GlobalProcessDataPool {
            pool: Vec::new(),
            free_id: Vec::new(),
        }
    }

    fn shared(&self, process: &Process) -> Arc<ProcessShared> {
        Arc::clone(&self.pool[process.id])
    }

    fn find_by_id(&self, thread: &Thread) -> Process {
        // TODO: probably use a map on the pool instead, but HashSet on the process threads is prob
        // enough
        for (id, shared) in self.pool.iter().enumerate() {
            if shared.threads.lock().contains(thread) {
                return Process {
                    id,
                    signature: *shared.signature.lock(),
                };
            }
        }
        panic!("thread id {} doesn't belong to any processes", thread.id());
    }

    fn free(&mut self, process: Process) {
        self.free_id.push(process.id);
        // Signature 0 is always invalid
        *self.pool[process.id].signature.lock() = 0;
    }

    fn alloc(&mut self) -> Process {
        if let Some(id) = self.free_id.pop() {
            let free = &mut self.pool[id];
            let new = ProcessShared::new();
            let signature = *new.signature.lock();
            *free = Arc::new(new);

            return Process { id, signature };
        }

        let id = self.pool.len();
        let new = ProcessShared::new();
        let signature = *new.signature.lock();
        self.pool.push(Arc::new(new));

        Process { id, signature }
    }
}

struct ProcessShared {
    stacks: Mutex<StackAllocator>,
    threads: Mutex<HashSet<Thread>>,
    signature: Mutex<usize>,
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
            threads: HashSet::new().into(),
            signature: sig().into(),
        }
    }
}

#[derive(Clone, IPPacket)]
struct ExpandSharedPacket {
    table_template: Arc<InactivePageTable<RecurseLevel4LowerHalf>>,
}

static SIG: AtomicUsize = AtomicUsize::new(1);

fn sig() -> usize {
    SIG.fetch_add(1, Ordering::Relaxed)
}
