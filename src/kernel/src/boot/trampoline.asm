use16
org 0x8000
cli
cld
jmp 0:trampoline

GDT:
  dd 0
  dd 0

.code = $ - GDT
  dw 0xFFFF
  dw 0
  db 0
  db 10011010b
  db 11001111b
  db 0
.data = $ - GDT
  dw 0xFFFF
  dw 0
  db 0
  db 10010010b
  db 11001111b
  db 0
GDT_END:

GDT_PTR:
  dw GDT_END-GDT-1
  dd GDT

trampoline:
  xor ax, ax
  mov ds, ax
  mov ss, ax

  lgdt [GDT_PTR]

  mov eax, cr0 
  or al, 1       ; set PE (Protection Enable) bit in CR0 (Control Register 0)
  mov cr0, eax

  jmp GDT.code:protected_mode

use32
gdt64:
  dq 0 ; zero entry
.code = $ - gdt64 
  dq (1 shl 43) or (1 shl 44) or (1 shl 47) or (1 shl 53) ; code segment
.pointer:
  dw $ - gdt64 - 1
  dq gdt64
align 256
protected_mode:
  mov ax, GDT.data
	mov ds, ax
	mov ss, ax

  mov eax, dword [0x7000]
  or eax, 0x8
  mov cr3, eax

  mov     eax, cr4
  or      eax, 1 shl 5             ; set CR4.PAE
  mov     cr4, eax
  
  mov     ecx, 0xC0000080         ; MSR_EFER
  rdmsr
  or      eax, 1 shl 8             ; set EFER.LME
  or      eax, 1 shl 11            ; set EFER.NXE
  wrmsr

  mov     eax, cr0
  or      eax, 1 shl 31            ; set CR0.PG
  mov     cr0, eax

  lgdt [gdt64.pointer]

  jmp gdt64.code:trampoline_64

use64
trampoline_64:
  mov rax, 0xffff800000000000
  jmp rax
