ENTRY(boot_start)


PHDRS
{
    null              PT_NULL    FLAGS(0) ;                  	 /* Null segment */
    text              PT_LOAD    FLAGS((1 << 0) | (1 << 2)) ;	 /* Execute + Read */
    rodata            PT_LOAD    FLAGS((1 << 2)) ;           	 /* Read only */
    eh_frame          PT_LOAD    FLAGS((1 << 2)) ;
    eh_frame_hdr      PT_LOAD    FLAGS((1 << 2)) ;
    data              PT_LOAD    FLAGS((1 << 1) | (1 << 2)) ;	 /* Write + Read */
    bss               PT_LOAD    FLAGS((1 << 1) | (1 << 2)) ;	 /* Write + Read */
    trampoline        PT_LOAD    FLAGS((1 << 0) | (1 << 2)) ;	 /* Execute + Read */
    trampoline_data   PT_LOAD    FLAGS((1 << 2)) ; 		 /* Read only */
}

SECTIONS {
  . = 0xffff800000000000;
  
  .trampoline ALIGN(4K):
  {
    KEEP(*(.trampoline .trampoline.*))
    . = ALIGN(4K);
  } :trampoline

  .trampoline_data ALIGN(4K):
  {
    __trampoline_start = .;
    KEEP(*(.trampoline_data .trampoline_data.*))
    . = ALIGN(4K);
    __trampoline_end = .;
  } :trampoline_data

  .rodata ALIGN(4K):
  {
    *(.rodata .rodata.*)
    . = ALIGN(4K);
  } :rodata
  
  PROVIDE(__eh_frame = .);
  .eh_frame ALIGN(4K):
  {
    *(.eh_frame .eh_frame.*)
    . = ALIGN(4K);
  } :eh_frame

  .eh_frame_hdr ALIGN(4K):
  {
    *(.eh_frame_hdr .eh_frame_hdr.*)
    . = ALIGN(4K);
  } :eh_frame_hdr

  .text ALIGN(4K):
  {
    *(.text .text.*)
    __etext = .;
    . = ALIGN(4K);
  } :text

  .data ALIGN(4K):
  {
    *(.data .data.*)
    . = ALIGN(4K);
  } :data

  .bss ALIGN(4K):
  {
    *(.bss .bss.*)
    . = ALIGN(4K);
  } :bss
}
