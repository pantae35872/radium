use core::{
    mem::zeroed,
    sync::atomic::{AtomicUsize, Ordering},
};

use alloc::{boxed::Box, vec, vec::Vec};
use pager::{address::VirtAddr, registers::RFlagsFlags};
use sentinel::log;

use crate::{
    hlt_loop,
    initialization_context::{FinalPhase, InitializationContext},
    interrupt::FullInterruptStackFrame,
    memory::stack_allocator::Stack,
    smp::cpu_local,
};

use super::{driv_exit, SchedulerError};

#[derive(Debug)]
pub struct ThreadPool {
    pool: Vec<ThreadContext>,
    dead_thread: Vec<usize>,
}

#[derive(Debug)]
struct ThreadContext {
    alive: bool,
    stack: Stack,
}

#[derive(Debug)]
pub struct Thread {
    id: usize,
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
        unsafe { cpu_local().set_tid(self.id) };
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
        Thread {
            id: cpu_local().current_thread_id(),
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
    fn new<F>(f: F, id: usize, context: &ThreadContext) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let boxed: *mut F = Box::into_raw(f.into());
        let rdi = boxed as u64;

        Thread {
            id,
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

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn is_start(&self) -> bool {
        self.id == usize::MAX
    }

    pub fn is_bsp_thread(&self) -> bool {
        self.id == 0
    }

    pub fn is_halt_thread(&self) -> bool {
        self.id == 1
    }

    #[must_use]
    pub fn hlt_thread(stack: Stack) -> Self {
        Thread {
            id: 1,
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
        ctx: &mut InitializationContext<FinalPhase>,
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
            return Ok(Thread::new(f, dead, thread_ctx));
        }

        let new_context = ThreadPool::alloc_context(&mut cpu_local().ctx().lock())?;
        let id = self.pool.len();
        let thread = Thread::new(f, id, &new_context);
        self.pool.push(new_context);
        Ok(thread)
    }

    pub fn free(&mut self, thread: Thread) {
        assert!(
            thread.id != 0 || thread.id != 1,
            "Thread ID 0 and 1 should not be freed"
        );
        log!(
            Trace,
            "Freeing thread id: {}, On cpu: {}",
            thread.id,
            cpu_local().cpu_id()
        );
        self.dead_thread.push(thread.id);
        self.pool[thread.id].alive = false;
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
