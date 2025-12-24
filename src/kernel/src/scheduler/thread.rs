use core::{fmt::Display, mem::zeroed};

use alloc::{boxed::Box, vec, vec::Vec};
use pager::{address::VirtAddr, paging::table::RecurseLevel4UpperHalf, registers::RFlags};
use spin::RwLock;

use crate::{
    const_assert,
    gdt::CODE_SEG,
    hlt_loop,
    interrupt::{CORE_ID, ExtendedInterruptStackFrame},
    memory::stack_allocator,
    memory::stack_allocator::Stack,
    scheduler::CURRENT_THREAD_ID,
    smp::{CoreId, MAX_CPU},
    sync::spin_mpsc::SpinMPSC,
};

use super::{SchedulerError, driv_exit, thread_wait_exit};

static GLOBAL_THREAD_ID_MAP: RwLock<GlobalThreadIdPool> = RwLock::new(GlobalThreadIdPool::new());
static THREAD_HANDLE_POOL: RwLock<ThreadHandlePool> = RwLock::new(ThreadHandlePool::new());
static THREAD_MIGRATE_QUEUE: [SpinMPSC<(Thread, ThreadContext), 256>; MAX_CPU] =
    [const { SpinMPSC::new() }; MAX_CPU];

#[derive(Debug)]
struct ThreadHandlePool {
    pool: Vec<ThreadHandleData>,
    expire_handles: Vec<usize>,
}

impl ThreadHandlePool {
    const fn new() -> Self {
        Self {
            pool: Vec::new(),
            expire_handles: Vec::new(),
        }
    }

    fn is_expired(&self, handle: &ThreadHandle) -> bool {
        self.pool
            .get(handle.handle_id)
            .map(|e| e.expired || e.global_id != handle.global_id)
            .unwrap_or(true)
    }

    fn create(&mut self, global_id: usize) -> ThreadHandle {
        if let Some(expire) = self
            .expire_handles
            .pop_if(|e| self.pool[*e].global_id != global_id)
        {
            self.pool[expire].global_id = global_id;
            self.pool[expire].expired = false;

            return ThreadHandle {
                handle_id: expire,
                global_id,
            };
        }
        let id = self.pool.len();
        self.pool.push(ThreadHandleData {
            expired: false,
            global_id,
        });
        ThreadHandle {
            handle_id: id,
            global_id,
        }
    }

    fn free(&mut self, handle: usize) {
        self.pool[handle].expired = true;
        self.expire_handles.push(handle);
    }
}

#[derive(Debug)]
struct ThreadHandleData {
    expired: bool,
    global_id: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadHandle {
    handle_id: usize,
    global_id: usize,
}

impl ThreadHandle {
    pub fn into_raw(self) -> (usize, usize) {
        (self.handle_id, self.global_id)
    }

    /// Must be called with the value from [Self::into_raw]
    pub unsafe fn from_raw(handle_id: usize, global_id: usize) -> Self {
        Self {
            handle_id,
            global_id,
        }
    }

    #[inline]
    pub fn id(&self) -> Option<usize> {
        let pool = THREAD_HANDLE_POOL.read();
        if pool.is_expired(self) {
            return None;
        }

        Some(pool.pool[self.handle_id].global_id)
    }

    pub fn join(self) {
        if !THREAD_HANDLE_POOL.read().is_expired(&self) {
            thread_wait_exit(self.global_id);
        }
    }
}

#[derive(Debug)]
pub struct ThreadPool {
    pool: Vec<ThreadContext>,
    dead_thread: Vec<usize>,
    invalid_thread: Vec<usize>,
}

#[derive(Debug)]
struct GlobalThreadIdPool {
    pool: Vec<GlobalThreadIdData>,
    free_id: Vec<usize>,
}

#[derive(Debug)]
struct GlobalThreadIdData {
    local_id: LocalThreadId,
    handle_id: usize,
}

impl GlobalThreadIdPool {
    pub const fn new() -> Self {
        Self {
            pool: Vec::new(),
            free_id: Vec::new(),
        }
    }

    #[inline]
    fn translate(&self, global_id: usize) -> LocalThreadId {
        self.pool[global_id].local_id
    }

    fn migrate(&mut self, global_id: usize, new_local_id: LocalThreadId) {
        self.pool[global_id].local_id = new_local_id;
    }

    fn alloc(&mut self, local_id: LocalThreadId) -> (usize, ThreadHandle) {
        if let Some(free) = self.free_id.pop() {
            let id = &mut self.pool[free];

            let handle = THREAD_HANDLE_POOL.write().create(free);
            id.local_id = local_id;
            id.handle_id = handle.handle_id;

            return (free, handle);
        }

        let id = self.pool.len();
        let handle = THREAD_HANDLE_POOL.write().create(id);
        self.pool.push(GlobalThreadIdData {
            local_id,
            handle_id: handle.handle_id,
        });
        (id, handle)
    }

    fn free(&mut self, global_id: usize) -> LocalThreadId {
        self.free_id.push(global_id);
        THREAD_HANDLE_POOL
            .write()
            .free(self.pool[global_id].handle_id);
        self.pool[global_id].local_id
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LocalThreadId {
    core: CoreId,
    thread: u32,
}

const_assert!(size_of::<LocalThreadId>() == size_of::<u64>() * 2);

#[derive(Debug)]
struct ThreadContext {
    alive: bool,
    pinned: bool,
    stack: Stack,
}

#[repr(C)]
#[derive(Debug)]
pub struct ThreadState {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub instruction_pointer: VirtAddr,
    pub code_segment: u64,
    pub cpu_flags: RFlags,
    pub stack_pointer: VirtAddr,
    pub stack_segment: u64,
}

#[repr(C)]
#[derive(Debug)]
pub struct Thread {
    global_id: usize,
    local_id: LocalThreadId,
    pub state: ThreadState,
}

impl ThreadState {
    pub fn restore(self, stack_frame: &mut ExtendedInterruptStackFrame) {
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

    pub fn capture(stack_frame: &ExtendedInterruptStackFrame) -> Self {
        Self {
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
    fn new<F>(f: F, context: &ThreadContext) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let boxed: *mut F = Box::into_raw(f.into());
        let rdi = boxed as u64;

        Self {
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
            instruction_pointer: VirtAddr::new(thread_trampoline::<F> as *const () as u64),
            code_segment: CODE_SEG.0.into(),
            cpu_flags: RFlags::ID | RFlags::AlignmentCheck | RFlags::InterruptEnable,
            stack_pointer: context.stack.top(),
            stack_segment: 0,
        }
    }

    #[must_use]
    pub fn hlt_thread(stack: Stack) -> Self {
        Self {
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
            instruction_pointer: VirtAddr::new(hlt_loop as *const () as u64), // HLT thread is unaligned (should
            // be fine tho)
            code_segment: 8, // FIXME: I'm too lazy i'll just assume it's eight
            cpu_flags: RFlags::InterruptEnable,
            stack_pointer: stack.top(),
            stack_segment: 0,
        }
    }
}

impl Thread {
    pub fn restore(self, stack_frame: &mut ExtendedInterruptStackFrame) {
        *CURRENT_THREAD_ID.inner_mut() = self.global_id;
        self.state.restore(stack_frame);
    }

    pub fn capture(stack_frame: &ExtendedInterruptStackFrame) -> Self {
        let global_id = *CURRENT_THREAD_ID;
        Thread {
            global_id,
            local_id: GLOBAL_THREAD_ID_MAP.read().translate(global_id),
            state: ThreadState::capture(stack_frame),
        }
    }

    #[must_use]
    fn new<F>(f: F, local_id: LocalThreadId, context: &ThreadContext) -> (Self, ThreadHandle)
    where
        F: FnOnce() + Send + 'static,
    {
        let (global_id, handle) = GLOBAL_THREAD_ID_MAP.write().alloc(local_id);

        (
            Thread {
                global_id,
                local_id,
                state: ThreadState::new(f, context),
            },
            handle,
        )
    }

    pub fn migrate(&mut self, new_local_thread: u32) {
        let new_local = LocalThreadId {
            core: *CORE_ID,
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

    // # Safety
    // the caller must uphold a contract with an interrupt invoker that rcx is the pointer to the
    // first argument
    pub unsafe fn read_first_arg_rsi<T>(&self) -> T {
        unsafe { core::ptr::read(self.state.rsi as *const T) }
    }

    // # Safety
    // the caller must uphold a contract with an interrupt invoker that rcx is the return value
    // addres
    pub unsafe fn write_return_rcx<T>(&mut self, obj: T) {
        unsafe {
            core::ptr::write(self.state.rcx as *mut T, obj);
        }
    }

    #[must_use]
    pub fn hlt_thread(stack: Stack, core: CoreId) -> Self {
        let local_id = LocalThreadId { core, thread: 1 };
        let (global_id, _handle) = GLOBAL_THREAD_ID_MAP.write().alloc(local_id);
        Thread {
            global_id,
            local_id,
            state: ThreadState::hlt_thread(stack),
        }
    }
}

pub fn global_id_to_local_id(global_id: usize) -> LocalThreadId {
    GLOBAL_THREAD_ID_MAP.read().translate(global_id)
}

impl LocalThreadId {
    pub fn create_local(thread: u32) -> Self {
        Self {
            core: *CORE_ID,
            thread,
        }
    }

    pub fn core(&self) -> CoreId {
        self.core
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

impl ThreadPool {
    /// Create new thread pool, fails if failed to allocate the context for the hlt thread
    pub fn new() -> Self {
        Self {
            pool: vec![unsafe { zeroed() }, unsafe { zeroed() }],
            dead_thread: Vec::new(),
            invalid_thread: Vec::new(),
        }
    }

    fn alloc_context() -> Result<ThreadContext, SchedulerError> {
        Ok(ThreadContext {
            alive: true,
            pinned: false,
            stack: stack_allocator::<RecurseLevel4UpperHalf, _>(|mut s| s.alloc_stack(256))
                .ok_or(SchedulerError::FailedToAllocateStack { size: 256 })?,
        })
    }

    pub fn alloc<F>(&mut self, f: F) -> Result<(Thread, ThreadHandle), SchedulerError>
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
            return Ok(Thread::new(
                f,
                LocalThreadId::create_local(dead as u32),
                thread_ctx,
            ));
        }

        if let Some(id) = self.invalid_thread.pop() {
            let new_context = ThreadPool::alloc_context()?;
            let thread = Thread::new(f, LocalThreadId::create_local(id as u32), &new_context);
            self.pool[id] = new_context;
            return Ok(thread);
        }

        let new_context = ThreadPool::alloc_context()?;
        let id = self.pool.len();
        let thread = Thread::new(f, LocalThreadId::create_local(id as u32), &new_context);
        self.pool.push(new_context);
        Ok(thread)
    }

    pub fn check_migrate(&mut self, mut callback: impl FnMut(Thread)) {
        while let Some((mut thread, thread_ctx)) = THREAD_MIGRATE_QUEUE[CORE_ID.id()].pop() {
            if let Some(id) = self.invalid_thread.pop() {
                thread.migrate(id as u32);
                assert!(thread_ctx.alive);
                self.pool[id] = thread_ctx;
                callback(thread);
            } else {
                let id = self.pool.len();
                thread.migrate(id as u32);
                assert!(thread_ctx.alive);
                self.pool.push(thread_ctx);
                callback(thread);
            }
        }
    }

    pub fn pin(&mut self, thread: &Thread) {
        assert!(thread.local_id().core == *CORE_ID);

        self.pool[thread.local_id().thread as usize].pinned = true;
    }

    pub fn unpin(&mut self, thread: &Thread) {
        assert!(thread.local_id().core == *CORE_ID);

        self.pool[thread.local_id().thread as usize].pinned = false;
    }

    pub fn is_pinned(&mut self, thread: &Thread) -> bool {
        assert!(thread.local_id().core == *CORE_ID);

        self.pool[thread.local_id().thread as usize].pinned
    }

    pub fn migrate(&mut self, dest: CoreId, mut migrate_thread: Thread) {
        self.invalid_thread
            .push(migrate_thread.local_id().thread as usize);
        let mut current_ctx = core::mem::replace(
            &mut self.pool[migrate_thread.local_id().thread as usize],
            // SAFETY: This is ok because we will never use invalid thread since we pushed it into
            // invalid thread list
            unsafe { zeroed() },
        );

        while let Err((m, c)) = THREAD_MIGRATE_QUEUE[dest.id()].push((migrate_thread, current_ctx))
        {
            (current_ctx, migrate_thread) = (c, m);
        }
    }

    pub fn free(&mut self, thread: Thread) {
        assert!(
            thread.local_id().core == *CORE_ID,
            "Thread has been migrated without changing the local id"
        );
        assert!(
            thread.local_id().thread != 0 || thread.local_id().thread != 1,
            "Thread ID 0 and 1 should not be freed"
        );
        self.dead_thread.push(thread.local_id().thread as usize);
        self.pool[thread.local_id().thread as usize].alive = false;
        self.pool[thread.local_id().thread as usize].pinned = false;
        GLOBAL_THREAD_ID_MAP.write().free(thread.global_id());
    }
}

#[unsafe(naked)]
unsafe extern "C" fn thread_trampoline<F>(f_ptr: *mut F)
where
    F: FnOnce(),
{
    core::arch::naked_asm!(
        "
        call {tramp}
        ",
        tramp = sym thread_trampoline_inner::<F>,
    )
}

extern "C" fn thread_trampoline_inner<F>(f_ptr: *mut F)
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
