global boot_start
global temporary_p4_table
global early_alloc
extern start

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
early_alloc:
  resb 4096 * 64
stack_bottom:
  resb 1024 * 1024 ; 1M Stack
stack_top:
