use core::{error::Error, fmt::Display, mem::zeroed};

use alloc::{boxed::Box, vec, vec::Vec};
use pager::{address::VirtAddr, registers::RFlagsFlags};
use sentinel::log;
use spin::{Mutex, RwLock};

use crate::{
    const_assert, hlt_loop,
    initialization_context::{End, InitializationContext},
    interrupt::{FullInterruptStackFrame, InterruptIndex},
    memory::stack_allocator::Stack,
    serial_println,
    smp::{cpu_id_to_apic_id, cpu_local, MAX_CPU},
};

use super::{driv_exit, SchedulerError};

static GLOBAL_THREAD_ID_MAP: RwLock<GlobalThreadIdPool> = RwLock::new(GlobalThreadIdPool::new());
static THREAD_MIGRATE_QUEUE: [Mutex<Option<(Thread, ThreadContext)>>; MAX_CPU] =
    [const { Mutex::new(None) }; MAX_CPU];

#[derive(Debug)]
pub struct ThreadPool {
    pool: Vec<ThreadContext>,
    dead_thread: Vec<usize>,
}

#[derive(Debug)]
struct GlobalThreadIdPool {
    pool: Vec<LocalThreadId>,
    free_id: Vec<usize>,
}

#[derive(Debug, Clone, Copy)]
pub struct LocalThreadId {
    core: u32,
    thread: u32,
}

const_assert!(size_of::<LocalThreadId>() == size_of::<u64>());

#[derive(Debug)]
struct ThreadContext {
    alive: bool,
    stack: Stack,
}

#[derive(Debug)]
pub struct Thread {
    global_id: usize,
    local_id: LocalThreadId,
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rsi: u64,
    rdi: u64,
    rbp: u64,
    rdx: u64,
    rcx: u64,
    rbx: u64,
    rax: u64,
    instruction_pointer: VirtAddr,
    code_segment: u64,
    cpu_flags: RFlagsFlags,
    stack_pointer: VirtAddr,
    stack_segment: u64,
}

impl Thread {
    pub fn restore(self, stack_frame: &mut FullInterruptStackFrame) {
        // SAFETY: This is safe because thread can only be created in this module
        unsafe { cpu_local().set_tid(self.global_id) };
        stack_frame.r15 = self.r15;
        stack_frame.r14 = self.r14;
        stack_frame.r13 = self.r13;
        stack_frame.r12 = self.r12;
        stack_frame.r11 = self.r11;
        stack_frame.r10 = self.r10;
        stack_frame.r9 = self.r9;
        stack_frame.r8 = self.r8;
        stack_frame.rsi = self.rsi;
        stack_frame.rdi = self.rdi;
        stack_frame.rbp = self.rbp;
        stack_frame.rdx = self.rdx;
        stack_frame.rcx = self.rcx;
        stack_frame.rbx = self.rbx;
        stack_frame.rax = self.rax;
        stack_frame.instruction_pointer = self.instruction_pointer;
        stack_frame.code_segment = self.code_segment;
        stack_frame.cpu_flags = self.cpu_flags;
        stack_frame.stack_pointer = self.stack_pointer;
        stack_frame.stack_segment = self.stack_segment;
    }

    pub fn capture(stack_frame: &FullInterruptStackFrame) -> Self {
        let global_id = cpu_local().current_thread_id();
        Thread {
            global_id,
            local_id: GLOBAL_THREAD_ID_MAP.read().translate(global_id),
            r15: stack_frame.r15,
            r14: stack_frame.r14,
            r13: stack_frame.r13,
            r12: stack_frame.r12,
            r11: stack_frame.r11,
            r10: stack_frame.r10,
            r9: stack_frame.r9,
            r8: stack_frame.r8,
            rsi: stack_frame.rsi,
            rdi: stack_frame.rdi,
            rbp: stack_frame.rbp,
            rdx: stack_frame.rdx,
            rcx: stack_frame.rcx,
            rbx: stack_frame.rbx,
            rax: stack_frame.rax,
            instruction_pointer: stack_frame.instruction_pointer,
            code_segment: stack_frame.code_segment,
            cpu_flags: stack_frame.cpu_flags,
            stack_pointer: stack_frame.stack_pointer,
            stack_segment: stack_frame.stack_segment,
        }
    }

    #[must_use]
    fn new<F>(f: F, local_id: LocalThreadId, context: &ThreadContext) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let boxed: *mut F = Box::into_raw(f.into());
        let rdi = boxed as u64;

        let global_id = GLOBAL_THREAD_ID_MAP.write().alloc(local_id);

        Thread {
            global_id,
            local_id,
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rsi: 0,
            rdi,
            rbp: context.stack.bottom().as_u64(),
            rdx: 0,
            rcx: 0,
            rbx: 0,
            rax: 0,
            instruction_pointer: VirtAddr::new(thread_trampoline::<F> as u64),
            code_segment: cpu_local().code_seg().0.into(),
            cpu_flags: RFlagsFlags::ID | RFlagsFlags::AlignmentCheck | RFlagsFlags::InterruptEnable,
            stack_pointer: context.stack.top(),
            stack_segment: 0,
        }
    }

    pub fn migrate(&mut self, new_local_thread: u32) {
        let new_local = LocalThreadId {
            core: cpu_local().cpu_id() as u32,
            thread: new_local_thread,
        };
        GLOBAL_THREAD_ID_MAP
            .write()
            .migrate(self.global_id(), new_local);
        self.local_id = new_local;
    }

    #[inline]
    pub fn local_id(&self) -> LocalThreadId {
        self.local_id
    }

    #[inline]
    pub fn global_id(&self) -> usize {
        self.global_id
    }

    #[must_use]
    pub fn hlt_thread(stack: Stack, core: usize) -> Self {
        let local_id = LocalThreadId {
            core: core as u32,
            thread: 1,
        };
        let global_id = GLOBAL_THREAD_ID_MAP.write().alloc(local_id);
        Thread {
            global_id,
            local_id,
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rsi: 0,
            rdi: 0,
            rbp: stack.bottom().as_u64(),
            rdx: 0,
            rcx: 0,
            rbx: 0,
            rax: 0,
            instruction_pointer: VirtAddr::new(hlt_loop as u64),
            code_segment: 8, // FIXME: I'm too lazy i'll just assume it's eight
            cpu_flags: RFlagsFlags::InterruptEnable,
            stack_pointer: stack.top(),
            stack_segment: 0,
        }
    }
}

impl LocalThreadId {
    pub fn create_local(thread: u32) -> Self {
        Self {
            core: cpu_local().cpu_id() as u32,
            thread,
        }
    }

    pub fn is_bsp_thread(&self) -> bool {
        self.thread == 0
    }

    pub fn is_halt_thread(&self) -> bool {
        self.thread == 1
    }
}

impl Display for LocalThreadId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "`Local Thread ID: {} on core: {}`",
            self.thread, self.core
        )
    }
}

#[derive(Debug)]
pub enum ThreadMigrationError {
    ThreadQueueFull,
    ThreadQueueLocked,
    SchedulerError(SchedulerError),
}

impl Display for ThreadMigrationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ThreadQueueFull => write!(f, "Thread queue full"),
            Self::ThreadQueueLocked => write!(f, "Thread queue locked"),
            Self::SchedulerError(err) => write!(f, "{err}"),
        }
    }
}

impl From<SchedulerError> for ThreadMigrationError {
    fn from(value: SchedulerError) -> Self {
        Self::SchedulerError(value)
    }
}

impl Error for ThreadMigrationError {}

impl GlobalThreadIdPool {
    pub const fn new() -> Self {
        Self {
            pool: Vec::new(),
            free_id: Vec::new(),
        }
    }

    #[inline]
    fn translate(&self, global_id: usize) -> LocalThreadId {
        self.pool[global_id]
    }

    fn migrate(&mut self, global_id: usize, new_local_id: LocalThreadId) {
        self.pool[global_id] = new_local_id;
    }

    fn alloc(&mut self, local_id: LocalThreadId) -> usize {
        if let Some(free) = self.free_id.pop() {
            let id = &mut self.pool[free];
            *id = local_id;

            log!(
                Trace,
                "Thread ID Pool is giving out global thread id: `{free}` for local thread id: {local_id}"
            );

            return free;
        }

        let id = self.pool.len();
        self.pool.push(local_id);
        log!(
            Trace,
            "Thread ID Pool is giving out global thread id: `{id}` for local thread id: {local_id}"
        );
        id
    }

    fn free(&mut self, global_id: usize) -> LocalThreadId {
        self.free_id.push(global_id);
        log!(
            Trace,
            "Thread ID Pool is freeing global id: `{}`",
            global_id
        );
        return self.pool[global_id];
    }
}

impl ThreadPool {
    /// Create new thread pool, fails if failed to allocate the context for the hlt thread
    pub fn new() -> Self {
        log!(Debug, "Creating new thread pool");
        Self {
            pool: vec![unsafe { zeroed() }, unsafe { zeroed() }],
            dead_thread: Vec::new(),
        }
    }

    fn alloc_context(
        ctx: &mut InitializationContext<End>,
    ) -> Result<ThreadContext, SchedulerError> {
        log!(Trace, "Allocating new thread context");
        Ok(ThreadContext {
            alive: true,
            stack: ctx
                .stack_allocator()
                .alloc_stack(256)
                .ok_or(SchedulerError::FailedToAllocateStack { size: 256 })?,
        })
    }

    #[must_use]
    pub fn alloc<F>(&mut self, f: F) -> Result<Thread, SchedulerError>
    where
        F: FnOnce() + Send + 'static,
    {
        if let Some(dead) = self.dead_thread.pop() {
            let thread_ctx = &mut self.pool[dead];
            assert!(
                !thread_ctx.alive,
                "Invalid state, there's alive thread in the dead thread pool"
            );
            thread_ctx.alive = true;
            // TODO: Clear previous thread data context for security
            return Ok(Thread::new(
                f,
                LocalThreadId::create_local(dead as u32),
                thread_ctx,
            ));
        }

        let new_context = ThreadPool::alloc_context(&mut cpu_local().ctx().lock())?;
        let id = self.pool.len();
        let thread = Thread::new(f, LocalThreadId::create_local(id as u32), &new_context);
        self.pool.push(new_context);
        Ok(thread)
    }

    pub fn check_migrate(&mut self) -> Option<Thread> {
        let (mut thread, thread_ctx) =
            match THREAD_MIGRATE_QUEUE[cpu_local().cpu_id()].lock().take() {
                Some(thread) => thread,
                None => return None,
            };

        let id = self.pool.len();
        thread.migrate(id as u32);
        assert!(thread_ctx.alive);
        self.pool.push(thread_ctx);
        Some(thread)
    }

    pub fn migrate(
        &mut self,
        cpu_id_destination: usize,
        migrate_thread: Thread,
    ) -> Result<(), ThreadMigrationError> {
        serial_println!(
            "Migrating thread {} to cpu {cpu_id_destination}",
            migrate_thread.global_id()
        );

        if THREAD_MIGRATE_QUEUE[cpu_id_destination].is_locked() {
            return Err(ThreadMigrationError::ThreadQueueLocked);
        }

        let mut thread = THREAD_MIGRATE_QUEUE[cpu_id_destination].lock();
        if thread.is_some() {
            return Err(ThreadMigrationError::ThreadQueueFull);
        }

        // FIXME: This create extra thread everytime the thread is migrated to a different core, we
        // need to mark the thread in the migrater as "available for replacement" and not allocate
        // extra context
        let mut new_ctx = ThreadPool::alloc_context(&mut cpu_local().ctx().lock())?;
        self.dead_thread
            .push(migrate_thread.local_id().thread as usize);
        new_ctx.alive = false;
        let current_ctx = core::mem::replace(
            &mut self.pool[migrate_thread.local_id().thread as usize],
            new_ctx,
        );

        *thread = Some((migrate_thread, current_ctx));

        drop(thread);

        cpu_local().lapic().send_fixed_ipi(
            cpu_id_to_apic_id(cpu_id_destination),
            InterruptIndex::ThreadMigrate,
        );

        Ok(())
    }

    pub fn free(&mut self, thread: Thread) {
        assert!(
            thread.local_id().core == cpu_local().cpu_id() as u32,
            "Thread has been migrated without changing the local id"
        );
        assert!(
            thread.local_id().thread != 0 || thread.local_id().thread != 1,
            "Thread ID 0 and 1 should not be freed"
        );
        log!(Debug, "Freeing thread: {}", thread.local_id());
        self.dead_thread.push(thread.local_id().thread as usize);
        self.pool[thread.local_id().thread as usize].alive = false;
        GLOBAL_THREAD_ID_MAP.write().free(thread.global_id());
    }
}

extern "C" fn thread_trampoline<F>(f_ptr: *mut F)
where
    F: FnOnce(),
{
    let f: Box<F> = unsafe { Box::from_raw(f_ptr) };
    f();
    driv_exit();
}

impl Default for ThreadPool {
    fn default() -> Self {
        Self::new()
    }
}
