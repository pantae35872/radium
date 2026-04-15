use core::arch::{asm, naked_asm};

use kernel_proc::{def_local, local_builder};
use pager::{
    address::VirtAddr,
    registers::{Efer, SystemCallLStar, SystemCallStar},
};

use crate::{
    gdt::{KERNEL_CODE_SEG, KERNEL_DATA_SEG, USER_CODE_SEG, USER_CODE_SEG_DUMMY, USER_DATA_SEG},
    initialization_context::{InitializationContext, Stage4},
    interrupt::{self, ExtendedInterruptStackFrame, HLT_STACK},
    memory::is_stack_aligned_16,
    userland::{
        self,
        pipeline::{CommonRequestContext, CommonRequestStackFrame, RequestReferer, dispatch::DispatchAction},
        syscall::SyscallId,
    },
};

def_local!(pub static IS_IN_SYSCALL: bool);

pub fn init(ctx: &mut InitializationContext<Stage4>) {
    ctx.local_initializer(|l| {
        l.register_after(|_| {
            assert_eq!(KERNEL_CODE_SEG.0 + 8, KERNEL_DATA_SEG.0, "Kernel code seg is not followed by kernel data seg");
            assert_eq!(
                USER_CODE_SEG_DUMMY.0 + 8,
                USER_DATA_SEG.0,
                "User code seg (dummy) is not followed by user data seg"
            );
            assert_eq!(
                USER_CODE_SEG_DUMMY.0 + 16,
                USER_CODE_SEG.0,
                "User code seg (dummy) is not followed by the real user code seg"
            );
            // SAFETY: The contract is checked above
            unsafe {
                Efer::SystemCallExtensions.write_retained();
                SystemCallStar { syscall_selector: *KERNEL_CODE_SEG, sysret_selector: *USER_CODE_SEG_DUMMY }.write();
                SystemCallLStar::write(VirtAddr::new(syscall_entry as *const () as u64));
            }
        });
        l.register(|builder, _context, _id| {
            local_builder!(builder, IS_IN_SYSCALL(false));
        });
    });
}

#[unsafe(no_mangle)]
extern "C" fn syscall_handler(stack_frame: &mut CommonRequestStackFrame) {
    *IS_IN_SYSCALL.inner_mut() = true;
    interrupt::enable();
    debug_assert!(is_stack_aligned_16(), "Unaligned stack in syscall handler");

    let id = SyscallId(stack_frame.rax as u32);
    let mut should_hlt = false;
    let mut use_iret = false;
    userland::pipeline::handle_request(
        CommonRequestContext::new(stack_frame, RequestReferer::SyscallRequest(id)),
        |CommonRequestContext { stack_frame, .. }, dispatcher| {
            dispatcher.dispatch(|action| match action {
                DispatchAction::HltLoop => {
                    should_hlt = true;
                }
                DispatchAction::ReplaceState(state) => {
                    stack_frame.replace_with(state);
                    use_iret = state.rcx != state.instruction_pointer.as_u64() || state.r11 != state.cpu_flags.bits();
                }
            })
        },
    );

    interrupt::disable();
    interrupt::drain_pending();
    *IS_IN_SYSCALL.inner_mut() = false;

    if should_hlt {
        let stack = HLT_STACK.top();
        // We can directly do hlt loop here since theres no requirement to return from,
        // the syscall instruction, and the stack will reset to a default value, when
        // the next syscall instruction is executed
        unsafe { asm!("mov rsp, {0}", "sti", "2:", "hlt", "jmp 2b", in(reg) stack.as_u64(), options(noreturn)) };
    }

    if use_iret {
        let mut iret_stack = ExtendedInterruptStackFrame {
            code_segment: USER_CODE_SEG.0.into(),
            stack_segment: USER_DATA_SEG.0.into(),
            ..Default::default()
        };
        iret_stack.replace_with(stack_frame);

        unsafe {
            asm! {
                "mov rsp, {0}",
                "pop r15",
                "pop r14",
                "pop r13",
                "pop r12",
                "pop r11",
                "pop r10",
                "pop r9",
                "pop r8",
                "pop rsi",
                "pop rdi",
                "pop rbp",
                "pop rdx",
                "pop rcx",
                "pop rbx",
                "pop rax",
                "swapgs",
                "iretq",

                in(reg) &iret_stack,
                options(noreturn)
            }
        };
    }
}

#[unsafe(no_mangle)]
#[unsafe(naked)]
extern "C" fn syscall_entry() {
    naked_asm! {
        "cli",
        "swapgs",
        "mov gs:8, rsp",
        "mov rsp, gs:0",
        "push gs:8", // gs:8 that we just saved is the user stack pointer
        "push r11", // r11 is rflags
        "push rcx", // rcx is rip
        "push rax",
        "push rbx",
        "push rcx",
        "push rdx",
        "push rbp",
        "push rdi",
        "push rsi",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov rdi, rsp",
        "call syscall_handler",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rsi",
        "pop rdi",
        "pop rbp",
        "pop rdx",
        "pop rcx",
        "pop rbx",
        "pop rax",
        "pop rcx", // sysret also use rcx as instruction pointer
        "pop r11", // sysret also use r11 as rflags
        "pop rsp",
        "swapgs",
        "sysretq",
    }
}
