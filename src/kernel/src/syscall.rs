use core::arch::naked_asm;

use pager::{
    address::VirtAddr,
    registers::{Efer, SystemCallLStar, SystemCallStar},
};

use crate::{
    gdt::{KERNEL_CODE_SEG, KERNEL_DATA_SEG, USER_CODE_SEG, USER_DATA_SEG},
    hlt_loop,
    initialization_context::{InitializationContext, Stage4},
    interrupt,
    userland::{
        self,
        pipeline::{
            dispatch::DispatchAction, CommonRequestContext, CommonRequestStackFrame, RequestReferer,
        },
        syscall::SyscallId,
    },
};

pub fn init(ctx: &mut InitializationContext<Stage4>) {
    ctx.local_initializer(|l| {
        l.register_after(|_| {
            assert_eq!(
                KERNEL_CODE_SEG.0 + 8,
                KERNEL_DATA_SEG.0,
                "Kernel code seg is not followed by kernel data seg"
            );
            assert_eq!(
                USER_CODE_SEG.0 + 8,
                USER_DATA_SEG.0,
                "User code seg is not followed by user data seg"
            );
            // SAFETY: The contract is checked above
            unsafe {
                Efer::SystemCallExtensions.write_retained();
                SystemCallStar {
                    syscall_selector: *KERNEL_CODE_SEG,
                    sysret_selector: *USER_CODE_SEG,
                }
                .write();
                SystemCallLStar::write(VirtAddr::new(syscall_entry as *const () as u64));
            }
        })
    });
}

#[unsafe(no_mangle)]
extern "C" fn syscall_handler(stack_frame: &mut CommonRequestStackFrame) {
    let id = SyscallId(stack_frame.rax as u32);
    let mut should_hlt = false;
    userland::pipeline::handle_request(
        CommonRequestContext::new(stack_frame, RequestReferer::SyscallRequest(id)),
        |CommonRequestContext { stack_frame, .. }, dispatcher| {
            dispatcher.dispatch(|action| match action {
                DispatchAction::HltLoop => {
                    should_hlt = true;
                }
                DispatchAction::ReplaceState(state) => {
                    stack_frame.replace_with(state);
                }
            })
        },
    );

    if should_hlt {
        // We can directly do hlt loop here since theres no requirement to return from,
        // the syscall instruction, and the stack will reset to a default value, when
        // the next syscall instruction is executed
        interrupt::enable();
        hlt_loop();
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
