use core::{
    arch::asm,
    cmp::Reverse,
    error::Error,
    fmt::Display,
    hint::unreachable_unchecked,
    sync::atomic::{AtomicUsize, Ordering},
};

use alloc::{
    collections::{binary_heap::BinaryHeap, vec_deque::VecDeque},
    vec::Vec,
};
use derivative::Derivative;
use hashbrown::HashMap;
use pager::address::VirtAddr;
use sentinel::log;
use thread::{Thread, ThreadPool};

use crate::{
    initialization_context::{End, InitializationContext},
    interrupt::{FullInterruptStackFrame, InterruptIndex},
    serial_println,
    smp::{cpu_local, CoreId, MAX_CPU},
    utils::spin_mpsc::SpinMPSC,
};

mod thread;

pub const DRIVCALL_SPAWN: u64 = 1;
pub const DRIVCALL_SLEEP: u64 = 2;
pub const DRIVCALL_EXIT: u64 = 3;
pub const DRIVCALL_FUTEX_WAIT: u64 = 4;
pub const DRIVCALL_FUTEX_WAKE: u64 = 5;

static THREAD_COUNT_EACH_CORE: [AtomicUsize; MAX_CPU] =
    [const { AtomicUsize::new(usize::MAX) }; MAX_CPU];

static FUTEX_CHECK: [SpinMPSC<VirtAddr, 256>; MAX_CPU] = [const { SpinMPSC::new() }; MAX_CPU];

// TODO: Implement the idea of custom syscall, worker threads
//static SYSCALL_MAP: [AtomicPtr<ThreadQueueNode>; 512] =
//    [const { AtomicPtr::new(core::ptr::null_mut()) }; 512];
//
//#[derive(Debug)]
//struct ThreadQueueNode {
//    thread: Thread,
//    next: AtomicPtr<ThreadQueueNode>,
//}
//
//#[derive(Debug)]
//struct ThreadQueueReceiver {
//    head: &'static ThreadQueueNode,
//}

#[derive(Derivative)]
#[derivative(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SleepEntry {
    wakeup_time: usize,
    #[derivative(PartialEq = "ignore", PartialOrd = "ignore", Ord = "ignore")]
    thread: Thread,
}

/// A scheduler that is specific to a cpu
pub struct LocalScheduler {
    hlt_thread: Option<Thread>,
    rr_queue: VecDeque<Thread>,
    sleep_queue: BinaryHeap<Reverse<SleepEntry>>,
    futex_map: HashMap<VirtAddr, Vec<Thread>>,
    timer_count: usize,
    scheduled_ms: usize,
    should_schedule: bool,
    pool: ThreadPool,
}

pub struct Dispatcher;

pub fn driv_exit() -> ! {
    unsafe {
        asm!("int 0x90", in("rdi") DRIVCALL_EXIT); // Do a driv call
        unreachable_unchecked()
    }
}

impl LocalScheduler {
    pub fn new(ctx: &mut InitializationContext<End>, current_core_id: CoreId) -> Self {
        Self {
            rr_queue: VecDeque::new(),
            hlt_thread: Some(Thread::hlt_thread(
                ctx.stack_allocator()
                    .alloc_stack(2)
                    .expect("Failed to allocate stack for hlt thread"),
                current_core_id,
            )),
            sleep_queue: BinaryHeap::new(),
            should_schedule: false,
            futex_map: Default::default(),
            timer_count: 0,
            scheduled_ms: 0,
            pool: ThreadPool::new(),
        }
    }

    pub fn start_scheduling(&mut self) {
        self.should_schedule = true;
    }

    pub fn exit_thread(&mut self, thread: Thread) {
        self.pool.free(thread);
    }

    pub fn sleep_thread(&mut self, thread: Thread, amount_millis: usize) {
        let sleep_entry = SleepEntry {
            wakeup_time: self.timer_count + amount_millis,
            thread,
        };

        self.sleep_queue.push(Reverse(sleep_entry));
    }

    pub fn push_thread(&mut self, thread: Thread) {
        if thread.local_id().is_halt_thread() {
            self.hlt_thread = Some(thread);
        } else if self.should_schedule {
            self.rr_queue.push_back(thread);
        }
    }

    pub fn prepare_timer(&mut self) {
        self.timer_count += self.scheduled_ms;
        let tpms = cpu_local().ticks_per_ms();
        cpu_local().lapic().reset_timer(tpms * 10);
        self.scheduled_ms = 10;
    }

    pub fn check_migrate(&mut self) {
        match self.pool.check_migrate() {
            Some(thread) => {
                //serial_println!(
                //    "received thread {} on core {}",
                //    thread.global_id(),
                //    cpu_local().core_id()
                //);
                self.rr_queue.push_back(thread);
            }
            None => log!(
                Warning,
                "Migrate interrupt received but no thread we placed on the queue"
            ),
        };
    }

    fn migrate_if_required(&mut self) {
        let local_core = cpu_local().core_id().id();
        let local_count = self.rr_queue.len();

        THREAD_COUNT_EACH_CORE[local_core].store(local_count, Ordering::Relaxed);

        let mut target_core = usize::MAX;
        let mut min_count = usize::MAX;

        for (core_id, count) in THREAD_COUNT_EACH_CORE.iter().enumerate() {
            let count = count.load(Ordering::Relaxed);

            if core_id == local_core || count == usize::MAX {
                continue;
            }

            if count < min_count {
                min_count = count;
                target_core = core_id;
            }
        }

        if target_core == usize::MAX || local_count <= min_count + 1 {
            return;
        }

        if let Some(thread) = self.rr_queue.pop_back() {
            let _ = self.pool.migrate(
                CoreId::new(target_core)
                    .expect("Unintialized core selected when calcuating thread migration"),
                thread,
            );
            THREAD_COUNT_EACH_CORE[local_core].fetch_sub(1, Ordering::Relaxed);
            THREAD_COUNT_EACH_CORE[target_core].fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn futex_wait(&mut self, addr: VirtAddr, thread: Thread, expected: usize) {
        if unsafe { addr.as_ptr::<AtomicUsize>().as_ref().unwrap() }.load(Ordering::SeqCst)
            != expected
        {
            self.rr_queue.push_front(thread);
        } else {
            self.futex_map
                .entry(addr)
                .or_insert(Default::default())
                .push(thread);
        }
    }

    pub fn check_futex(&mut self) {
        while let Some(futex) = FUTEX_CHECK[cpu_local().core_id().id()].pop() {
            self.futex_map.entry(futex).and_modify(|v| {
                v.drain(..v.len()).for_each(|e| self.rr_queue.push_back(e));
            });
        }
    }

    pub fn futex_wake(&mut self, addr: VirtAddr) {
        self.futex_map.entry(addr).and_modify(|v| {
            v.drain(..v.len()).for_each(|e| self.rr_queue.push_back(e));
            FUTEX_CHECK
                .iter()
                .enumerate()
                .filter(|(core, _)| {
                    cpu_local().core_id().id() != *core && *core < cpu_local().core_count()
                })
                .for_each(|(_, e)| e.push(addr).expect("FUTEX FULL"));
            cpu_local()
                .lapic()
                .broadcast_fixed_ipi(InterruptIndex::CheckFutex);
        });
    }

    pub fn schedule(&mut self) -> Option<Thread> {
        if !self.should_schedule {
            return None;
        }
        self.migrate_if_required();
        while let Some(sleep_thread) = self.sleep_queue.peek() {
            if self.timer_count >= sleep_thread.0.wakeup_time as usize {
                self.rr_queue
                    .push_back(self.sleep_queue.pop().unwrap().0.thread);
            } else {
                break;
            }
        }

        Some(
            self.rr_queue
                .pop_front()
                .unwrap_or_else(|| self.hlt_thread.take().unwrap()),
        )
    }

    pub fn spawn<F>(&mut self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let thread = Dispatcher::spawn(&mut self.pool, f).expect("Failed to spawn a thread");
        log!(
            Trace,
            "Spawned new thread Global ID: {}",
            thread.global_id()
        );
        self.rr_queue.push_back(thread);
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum SchedulerError {
    FailedToAllocateStack { size: usize },
}

pub fn sleep(in_millis: usize) {
    if cpu_local().current_thread_id() == 0 {
        panic!("Trying to use smart sleep, while in bsp thread");
    }

    unsafe {
        asm!("int 0x90", in("rdi") DRIVCALL_SLEEP, in("rax") in_millis); // Do a driv call
    }
}

/// Put the current thread into a sleep, until futex_wake is called;
///
/// # Safety
/// The caller must ensure that addr is a valid address
pub unsafe fn futex_wait(addr: VirtAddr, expected: usize) {
    unsafe {
        asm!("int 0x90", in("rdi") DRIVCALL_FUTEX_WAIT, in("rax") addr.as_u64(), in("rcx") expected);
    }
}

/// Wake all the thread waiting on this address
///
/// # Safety
/// The caller must ensure that addr is a valid address
pub unsafe fn futex_wake(addr: VirtAddr) {
    unsafe {
        asm!("int 0x90", in("rdi") DRIVCALL_FUTEX_WAKE, in("rax") addr.as_u64());
    }
}

impl Display for SchedulerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::FailedToAllocateStack { size } => {
                write!(f, "Scheduler failed to allocate stack with size: {size}")
            }
        }
    }
}

impl Error for SchedulerError {}

impl Dispatcher {
    pub fn spawn<F>(pool: &mut ThreadPool, f: F) -> Result<Thread, SchedulerError>
    where
        F: FnOnce(),
        F: Send + 'static,
    {
        pool.alloc(f)
    }

    pub fn dispatch(interrupt_stack_frame: &mut FullInterruptStackFrame, thread: Thread) {
        thread.restore(interrupt_stack_frame);
    }

    pub fn save(stack_frame: &FullInterruptStackFrame) -> Thread {
        Thread::capture(stack_frame)
    }
}

pub fn init(ctx: &mut InitializationContext<End>) {
    ctx.local_initializer(|i| {
        i.register(|builder, ctx, id| {
            builder.scheduler(LocalScheduler::new(ctx, id));
        })
    });
}
