use core::{
    arch::{asm, naked_asm},
    cmp::Reverse,
    error::Error,
    fmt::{Debug, Display},
    hint::unreachable_unchecked,
    num::NonZeroUsize,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicUsize, Ordering},
};

use alloc::{
    collections::{binary_heap::BinaryHeap, vec_deque::VecDeque},
    vec::Vec,
};
use conquer_once::spin::OnceCell;
use derivative::Derivative;
use hashbrown::HashMap;
use pager::address::VirtAddr;
use sentinel::log;
use thread::{global_id_to_local_id, Thread, ThreadMigrationError, ThreadPool};

use crate::{
    const_assert,
    initialization_context::{End, InitializationContext},
    interrupt::{FullInterruptStackFrame, InterruptIndex},
    smp::{cpu_local, CoreId, MAX_CPU},
    utils::spin_mpsc::SpinMPSC,
};

mod thread;

pub const MAX_VSYSCALL: usize = 64;
pub const VSYSCALL_REQUEST_RETRIES: usize = 32;

pub const DRIVCALL_ERR_VSYSCALL_FULL: u64 = 1 << 10;

pub const DRIVCALL_SPAWN: u64 = 1;
pub const DRIVCALL_SLEEP: u64 = 2;
pub const DRIVCALL_EXIT: u64 = 3;
pub const DRIVCALL_FUTEX_WAIT: u64 = 4;
pub const DRIVCALL_FUTEX_WAKE: u64 = 5;
pub const DRIVCALL_VSYS_REG: u64 = 6;
pub const DRIVCALL_VSYS_WAIT: u64 = 7;
pub const DRIVCALL_VSYS_REQ: u64 = 8;
pub const DRIVCALL_VSYS_RET: u64 = 9;

const MIGRATION_THRESHOLD: usize = 2;

static THREAD_COUNT_EACH_CORE: [AtomicUsize; MAX_CPU] =
    [const { AtomicUsize::new(usize::MAX) }; MAX_CPU];

static FUTEX_CHECK: [SpinMPSC<VirtAddr, 256>; MAX_CPU] = [const { SpinMPSC::new() }; MAX_CPU];

static VSYSCALL_REQUEST: [SpinMPSC<(usize, Thread), 256>; MAX_CPU] =
    [const { SpinMPSC::new() }; MAX_CPU];
static VSYSCALL_RETURN: [SpinMPSC<Thread, 256>; MAX_CPU] = [const { SpinMPSC::new() }; MAX_CPU];
static VSYSCALL_MAP: [OnceCell<usize>; MAX_VSYSCALL] = [const { OnceCell::uninit() }; MAX_VSYSCALL];

#[repr(C)]
#[derive(Debug)]
pub struct SomeLargeStructToTestInterruptRPC {
    pub number_start: u64,
    pub data: [usize; 64],
    pub number_end: u64,
}

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
    wake_marker: HashMap<VirtAddr, usize>,
    vsys_wait_request: HashMap<usize, Thread>,
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
            wake_marker: Default::default(),
            vsys_wait_request: Default::default(),
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
        self.pool
            .check_migrate(|thread| self.rr_queue.push_back(thread));
    }

    pub fn check_return(&mut self) {
        while let Some(thread) = VSYSCALL_RETURN[cpu_local().core_id().id()].pop() {
            self.rr_queue.push_back(thread);
        }
    }

    pub fn vsys_return_thread(&mut self, taker_thread: Thread) {
        let mut return_thread: Thread = unsafe { taker_thread.read_first_arg_rsi() };
        if return_thread.local_id().core() != cpu_local().core_id() {
            while let Err(thread) =
                VSYSCALL_RETURN[return_thread.local_id().core().id()].push(return_thread)
            {
                return_thread = thread;
            }
        } else {
            self.rr_queue.push_back(return_thread);
        }
    }

    pub fn check_vsys_request(&mut self) {
        let vsys = match VSYSCALL_REQUEST[cpu_local().core_id().id()].peek() {
            Some((vsys, _)) => vsys,
            None => return,
        };
        if self.vsys_wait_request.contains_key(vsys) {
            let (vsys, requester) = VSYSCALL_REQUEST[cpu_local().core_id().id()]
                .pop()
                .expect("This should success because of peek guard above");

            let mut thread = self
                .vsys_wait_request
                .remove(&vsys)
                .expect("Should success because of contains_key above");

            // SAFETY: the vsys wait driver call return thread is provided through rcx
            unsafe { thread.write_return_rcx(requester) }

            self.rr_queue.push_back(thread);
        }
    }

    pub fn vsys_reg(&mut self, syscall: usize, thread_id: usize) {
        VSYSCALL_MAP[syscall].init_once(|| thread_id);
    }

    pub fn vsys_wait(&mut self, syscall: usize, thread: Thread) {
        self.vsys_wait_request.insert(syscall, thread);
        self.check_vsys_request();
    }

    pub fn vsys_req(&mut self, syscall: usize, mut thread: Thread) {
        if let Some(worker) = VSYSCALL_MAP[syscall].get() {
            let id = global_id_to_local_id(*worker).core();
            let mut timeout = 0;
            while let Err((_, mut t)) = VSYSCALL_REQUEST[id.id()].push((syscall, thread)) {
                if id == cpu_local().core_id() {
                    self.check_vsys_request();
                }
                if timeout > VSYSCALL_REQUEST_RETRIES {
                    t.state.rdi = DRIVCALL_ERR_VSYSCALL_FULL;
                    self.rr_queue.push_back(t);
                    break;
                }
                thread = t;
                timeout += 1;
            }
        }
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

        if target_core == usize::MAX || local_count <= min_count + MIGRATION_THRESHOLD {
            return;
        }

        if let Some(thread) = self.rr_queue.pop_back() {
            let core = CoreId::new(target_core)
                .expect("Unintialized core selected when calcuating thread migration");
            log!(
                Trace,
                "Migrating thread {} to core {}",
                thread.global_id(),
                core
            );
            self.pool.migrate(core, thread);

            THREAD_COUNT_EACH_CORE[local_core].fetch_sub(1, Ordering::Relaxed);
            THREAD_COUNT_EACH_CORE[target_core].fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn futex_wait(&mut self, addr: VirtAddr, thread: Thread, expected: usize) {
        let val = unsafe { addr.as_ptr::<AtomicUsize>().as_ref().unwrap() };

        if val.load(Ordering::SeqCst) != expected
            && self
                .wake_marker
                .get(&addr)
                .is_some_and(|e| matches!(e.checked_sub(1), Some(_)))
        {
            self.wake_marker.entry(addr).and_modify(|e| *e -= 1);
            self.rr_queue.push_front(thread);
            return;
        }

        self.futex_map.entry(addr).or_default().push(thread)
    }

    pub fn check_futex(&mut self) {
        while let Some(futex) = FUTEX_CHECK[cpu_local().core_id().id()].pop() {
            let mut any_woken = false;
            self.futex_map.entry(futex).and_modify(|v| {
                if let Some(e) = v.pop() {
                    self.rr_queue.push_front(e);
                    any_woken = true;
                }
            });
            if !any_woken {
                *self.wake_marker.entry(futex).or_default() += 1;
            }
        }
    }

    pub fn futex_wake(&mut self, addr: VirtAddr) {
        let mut any_woken = false;

        self.futex_map.entry(addr).and_modify(|v| {
            if let Some(e) = v.pop() {
                self.rr_queue.push_front(e);
                any_woken = true;
            }
        });

        if any_woken {
            return;
        }

        FUTEX_CHECK
            .iter()
            .enumerate()
            .filter(|(core, _)| {
                cpu_local().core_id().id() != *core && *core < cpu_local().core_count()
            })
            .for_each(|(c, e)| {
                while e.push(addr).is_err() {
                    cpu_local()
                        .lapic()
                        .send_fixed_ipi(CoreId::new(c).unwrap(), InterruptIndex::CheckFutex);
                }
            });

        cpu_local()
            .lapic()
            .broadcast_fixed_ipi(InterruptIndex::CheckFutex);

        *self.wake_marker.entry(addr).or_default() += 1;
    }

    pub fn schedule(&mut self) -> Option<Thread> {
        if !self.should_schedule {
            return None;
        }
        self.migrate_if_required();
        while let Some(sleep_thread) = self.sleep_queue.peek() {
            if self.timer_count >= sleep_thread.0.wakeup_time as usize {
                self.rr_queue
                    .push_front(self.sleep_queue.pop().unwrap().0.thread);
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

/// A simple wrapper around vsys driv call automatically return a thread
#[repr(transparent)]
#[derive(Debug)]
pub struct VsysThread(Thread);

impl VsysThread {
    pub fn new(number: usize) -> Self {
        Self(vsys_wait(number))
    }
}

impl DerefMut for VsysThread {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Deref for VsysThread {
    type Target = Thread;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for VsysThread {
    fn drop(&mut self) {
        vsys_ret(&self);
    }
}

#[unsafe(naked)]
extern "C" fn vsys_ret(thread: &Thread) {
    unsafe {
        naked_asm!(
            "mov rsi, rdi", // RDI - the first argument : Since rdi is being use for driver call number we use rcx instead
            "mov rdi, {drivcall}",
            "int 0x90",
            "ret",
            drivcall = const DRIVCALL_VSYS_RET,
        );
    }
}

/// Register the current thraed to a vsyscall
#[unsafe(naked)]
extern "C" fn vsys_wait(number: usize) -> Thread {
    unsafe {
        naked_asm!(
            "mov rcx, rdi", // RDI - return value : Since rdi is being use for driver call number we use rcx instead
            "mov rax, rsi", // RSI - first argument
            "mov rdi, {drivcall}",
            "int 0x90",
            "ret",
            drivcall = const DRIVCALL_VSYS_WAIT,
        );
    }
}

/// Request a vsys call
pub fn vsys_req(number: usize) {
    unsafe {
        asm!("int 0x90", in("rdi") DRIVCALL_VSYS_REQ, in("rax") number);
    }
}

/// Register the current thraed to a vsyscall
pub fn vsys_reg(number: usize) {
    unsafe {
        asm!("int 0x90", in("rdi") DRIVCALL_VSYS_REG, in("rax") number);
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
