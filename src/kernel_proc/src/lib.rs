use proc_macro::TokenStream;
use quote::{format_ident, quote};

#[proc_macro]
pub fn fill_idt(_item: TokenStream) -> TokenStream {
    let handlers = (32..255).map(|interrupt_number| {
        let interrupt_number = interrupt_number as usize;
        let fn_name = format_ident!("gen_interrupt_{}", interrupt_number);
        quote! {
            unsafe { idt[#interrupt_number]
                .set_handler_addr(VirtAddr::new(#fn_name as u64))
                .set_stack_index(GENERAL_STACK_INDEX) };
        }
    });
    let output = quote! {
        #(#handlers)*
    };
    output.into()
}

#[proc_macro]
pub fn generate_interrupt_handlers(_item: TokenStream) -> TokenStream {
    let handlers = (32..255)
        .map(|interrupt_number| {
            let interrupt_number = interrupt_number as usize;
            let fn_name = format_ident!("gen_interrupt_{}", interrupt_number);
            let interrupt_number_asm = format!("mov rsi, {interrupt_number}");
            quote! {
                #[unsafe(no_mangle)]
                #[unsafe(naked)]
                extern "C" fn #fn_name() {
                    unsafe {
                        core::arch::naked_asm!(
                            // Save general-purpose registers
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
                            #interrupt_number_asm,
                            "call external_interrupt_handler",
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
                            // Return from interrupt
                            "iretq",
                        );
                    }
                }
            }
        })
        .collect::<Vec<_>>();
    let output = quote! {
        #(#handlers)*
    };
    output.into()
}
