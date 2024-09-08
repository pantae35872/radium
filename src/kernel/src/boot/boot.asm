global boot_start
global p4_table
extern start

section .text
bits 64
boot_start:
  ; setup the stack
  mov rsp, stack_top
  mov rbp, stack_bottom

  call set_up_page_tables
  call enable_paging

  ; clear segment register
  mov ax, 0
  mov ss, ax
  mov ds, ax
  mov es, ax
  mov fs, ax
  mov gs, ax

  jmp start ; jump to the rust kernel
  hlt ; should be unreachable

set_up_page_tables:
  mov rax, p4_table
  or rax, 0b11 
  mov [p4_table + 511 * 8], rax ; recersive mapped to self

  mov rax, p3_table
  or rax, 0b11 
  mov [p4_table], rax ; map p3_table to the first entry 

  mov rcx, 0 ; clear rcx (entry counter)

.map_p3_table:
  mov rsi, rcx
  shl rsi, 12 ; multiply by 4096 per table  
  lea rax, [p2_table + rsi] ; p2_table offset by rcx (rsi)
  or rax, 0b11 ; Read/Write | Present
  mov [p3_table + rcx * 8], rax ; 8 bytes per entry offset by rcx
  inc rcx ; increase 
  mov rdx, [rdi] ; get largest page
  cmp rcx, rdx ; compare if mapped all p3 entry
  jne .map_p3_table ; jump if not finish

  xor rbx, rbx ; clear rbx (rbx = 0) rbx is table_count
  mov rdx, p2_table ; pointer to a current p2_table being mapped offset by rbx
.map_p2_1g:
  xor rcx, rcx ; clear rcx (entry counter)

  .map_p2_table:
    lea rsi, [rdx + rcx * 8] ; 8 bytes per entry offset by rcx, current table pointer being mapped is store rdx
    imul rax, rcx, 0x200000 ; 2Mib mapped per entry multiply by rcx
    imul r10, rbx, 0x40000000 ; 1Gib per p2_table multiply by table_count 
    add rax, r10 ; offset an physical address by table
    or rax, 0b10000011 ; flags Huge Page | Read/Write | Present
    mov [rsi], rax ; Load address to a entry 

    inc rcx ; increase entry count      
    cmp rcx, 512 ; compare if mapped all entry in current table 512 * 2Mib = 1Gb per table
    jne .map_p2_table ; jump if not finish

  inc rbx ; increase table count
  mov rsi, rbx
  shl rsi, 12 ; multiply by 4096 byte per table
  lea rdx, [p2_table + rsi] ; load new pointer to a rdx register offset by rbx
  mov rax, [rdi] ; get largest page
  cmp rbx, rax ; compare if mapped all table
  jne .map_p2_1g ; jump if not finish
  ret

enable_paging:
  ; load p4 to cr3 register 
  mov rax, p4_table
  mov cr3, rax
  
  ret

section .bss
align 4096
p4_table:
  resb 4096
p3_table:
  resb 4096
p2_table:
  resb 4096 * 32
stack_bottom:
  resb 1024 * 256 ; 256 kb
stack_top:
