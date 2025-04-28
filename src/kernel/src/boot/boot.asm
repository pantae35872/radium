global boot_start
global temporary_p4_table
global early_alloc
extern start
extern ap_startup

section .trampoline
bits 64
from_long:
  mov rsp, qword [0x7010] 
  mov rbp, qword [0x7018] 

  ; The initialization context Arc
  mov rdi, qword [0x7020]

  mov rax, qword [0x7008]
  mov cr3, rax
  
  ; clear segment register
  mov ax, 0
  mov ss, ax
  mov ds, ax
  mov es, ax
  mov fs, ax
  mov gs, ax

  jmp ap_startup
  hlt

section .text
bits 64
boot_start:
  ; setup the stack

  mov rsp, stack_top
  mov rbp, stack_bottom

  ; clear segment register
  mov ax, 0
  mov ss, ax
  mov ds, ax
  mov es, ax
  mov fs, ax
  mov gs, ax

  jmp start ; jump to the rust kernel
  hlt ; should be unreachable

section .bss
align 4096
stack_bottom:
  resb 1024 * 1024 ; 1M Stack
stack_top:
