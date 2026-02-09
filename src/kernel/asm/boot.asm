format ELF64

public boot_start

extrn start
extrn ap_startup

; ---------------------------------------
; trampoline section
; ---------------------------------------

section '.trampoline' executable
use64

from_long:
    ; Getting data from in smp.rs
    ; #[repr(C)]
    ; struct SmpInitializationData {
    ;     page_table: u32,
    ;     _padding: u32, // Just to make it clear to me
    ;     real_page_table: u64,
    ;     stack: VirtAddr,
    ;     stack_bottom: VirtAddr,
    ;     ap_context: VirtAddr,
    ; }
    ; to ap_startup
    ; pub unsafe extern "C" fn ap_startup(ctx: *const ApInitializationContext) -> !

    ; Stack top 
    mov rax, 0x7010
    mov rsp, qword [rax]

    ; Stack bottom
    mov rax, 0x7018
    mov rbp, qword [rax]

    ; Initialization context
    mov rax, 0x7020
    mov rdi, qword [rax]
    
    ; Page table last
    mov rax, 0x7008
    mov rax, qword [rax]
    mov cr3, rax
    
    ; clear segment registers
    xor ax, ax
    mov ss, ax
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    
    jmp ap_startup
    hlt

; ---------------------------------------
; text
; ---------------------------------------

section '.text' executable
use64

boot_start:

    ; disable interrupts just in case UEFI didn't
    cli
    
    ; setup the stack
    mov rsp, stack_top
    mov rbp, stack_bottom
    
    ; clear segment registers
    xor ax, ax
    mov ss, ax
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    
    call start
    hlt

; ---------------------------------------
; bss
; ---------------------------------------

section '.bss' writeable align 4096

stack_bottom:
    rb 1024 * 1024        ; 1 MiB stack

stack_top:
