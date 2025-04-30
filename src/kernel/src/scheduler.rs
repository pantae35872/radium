use core::{
    arch::asm,
    cmp::Reverse,
    error::Error,
    fmt::Display,
    sync::atomic::{AtomicUsize, Ordering},
};

use alloc::{
    boxed::Box,
    collections::{binary_heap::BinaryHeap, vec_deque::VecDeque},
};
use derivative::Derivative;
use pager::{address::VirtAddr, registers::RFlagsFlags};

use crate::{
    hlt_loop,
    initialization_context::{InitializationContext, Phase3},
    interrupt::{FullInterruptStackFrame, TIMER_COUNT},
    memory::stack_allocator::Stack,
    serial_println,
    smp::cpu_local,
    PANIC_COUNT,
};

pub const DRIVCALL_SPAWN: u64 = 1;
pub const DRIVCALL_SLEEP: u64 = 2;

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

pub static THREAD_ID_COUNT: AtomicUsize = AtomicUsize::new(2);

pub struct Dispatcher;

extern "C" fn thread_trampoline<F>(f_ptr: *mut F)
where
    F: FnOnce(),
{
    let f: Box<F> = unsafe { Box::from_raw(f_ptr) };
    f();
    hlt_loop(); // TODO: Implement exit syscall
}

impl LocalScheduler {
    pub fn new(ctx: &mut InitializationContext<Phase3>) -> Self {
        Self {
            rr_queue: VecDeque::new(),
            hlt_thread: Some(Thread::hlt_thread(
                ctx.stack_allocator()
                    .alloc_stack(2)
                    .expect("Failed to allocate stack for hlt thread"),
            )),
            sleep_queue: BinaryHeap::new(),
        }
    }

    pub fn sleep_thread(&mut self, thread: Thread, amount_millis: usize) {
        let sleep_entry = SleepEntry {
            wakeup_time: TIMER_COUNT.load(Ordering::Relaxed) + amount_millis,
            thread,
        };

        self.sleep_queue.push(Reverse(sleep_entry));
    }

    pub fn push_thread(&mut self, thread: Thread, just_start: bool) {
        if thread.is_halt_thread() {
            self.hlt_thread = Some(thread);
        } else if !just_start {
            self.rr_queue.push_back(thread);
        }
    }

    pub fn schedule(&mut self) -> Thread {
        while let Some(sleep_thread) = self.sleep_queue.peek() {
            if TIMER_COUNT.load(Ordering::Relaxed) >= sleep_thread.0.wakeup_time as usize {
                self.rr_queue
                    .push_back(self.sleep_queue.pop().unwrap().0.thread);
            } else {
                break;
            }
        }

        self.rr_queue
            .pop_front()
            .unwrap_or_else(|| self.hlt_thread.take().unwrap())
    }

    pub fn spawn<F>(&mut self, f: F)
    where
        F: FnOnce(),
        F: Send + 'static,
    {
        let thread = Dispatcher::spawn(f).expect("Failed to spawn a thread");
        self.rr_queue.push_back(thread);
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum DispatcherError {
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

impl Display for DispatcherError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::FailedToAllocateStack { size } => {
                write!(f, "Dispatch failed to allocate stack with size: {size}")
            }
        }
    }
}

impl Error for DispatcherError {}

impl Thread {
    pub fn is_start(&self) -> bool {
        self.id == usize::MAX
    }

    pub fn is_bsp_thread(&self) -> bool {
        self.id == 0
    }

    pub fn is_halt_thread(&self) -> bool {
        self.id == 1
    }

    fn hlt_thread(stack: Stack) -> Self {
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

impl Dispatcher {
    pub fn spawn<F>(f: F) -> Result<Thread, DispatcherError>
    where
        F: FnOnce(),
        F: Send + 'static,
    {
        let stack = cpu_local()
            .ctx()
            .lock()
            .stack_allocator()
            .alloc_stack(256)
            .ok_or(DispatcherError::FailedToAllocateStack { size: 256 })?;
        let boxed: *mut F = Box::into_raw(f.into());
        let rdi = boxed as u64;

        Ok(Thread {
            id: THREAD_ID_COUNT.fetch_add(1, Ordering::Relaxed),
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
            rbp: stack.bottom().as_u64(),
            rdx: 0,
            rcx: 0,
            rbx: 0,
            rax: 0,
            instruction_pointer: VirtAddr::new(thread_trampoline::<F> as u64),
            code_segment: cpu_local().code_seg().0.into(),
            cpu_flags: RFlagsFlags::ID | RFlagsFlags::AlignmentCheck | RFlagsFlags::InterruptEnable,
            stack_pointer: stack.top(),
            stack_segment: 0,
        })
    }

    pub fn dispatch(interrupt_stack_frame: &mut FullInterruptStackFrame, thread: Thread) {
        // SAFETY: This is safe because thread can only be created in this module
        unsafe { cpu_local().set_tid(thread.id) };
        interrupt_stack_frame.r15 = thread.r15;
        interrupt_stack_frame.r14 = thread.r14;
        interrupt_stack_frame.r13 = thread.r13;
        interrupt_stack_frame.r12 = thread.r12;
        interrupt_stack_frame.r11 = thread.r11;
        interrupt_stack_frame.r10 = thread.r10;
        interrupt_stack_frame.r9 = thread.r9;
        interrupt_stack_frame.r8 = thread.r8;
        interrupt_stack_frame.rsi = thread.rsi;
        interrupt_stack_frame.rdi = thread.rdi;
        interrupt_stack_frame.rbp = thread.rbp;
        interrupt_stack_frame.rdx = thread.rdx;
        interrupt_stack_frame.rcx = thread.rcx;
        interrupt_stack_frame.rbx = thread.rbx;
        interrupt_stack_frame.rax = thread.rax;
        interrupt_stack_frame.instruction_pointer = thread.instruction_pointer;
        interrupt_stack_frame.code_segment = thread.code_segment;
        interrupt_stack_frame.cpu_flags = thread.cpu_flags;
        interrupt_stack_frame.stack_pointer = thread.stack_pointer;
        interrupt_stack_frame.stack_segment = thread.stack_segment;
    }

    pub fn save(stack_frame: &FullInterruptStackFrame) -> Thread {
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
}

pub fn init(ctx: &mut InitializationContext<Phase3>) {
    ctx.local_initializer(|i| {
        i.register(|builder, ctx, _id| {
            builder.scheduler(LocalScheduler::new(ctx));
        })
    });
}
