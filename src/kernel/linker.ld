ENTRY(boot_start)


PHDRS
{
    null    PT_NULL    FLAGS(0) ;                   /* Null segment */
    text    PT_LOAD    FLAGS((1 << 0) | (1 << 2)) ; /* Execute + Read */
    rodata  PT_LOAD    FLAGS((1 << 2)) ;            /* Read only */
    data    PT_LOAD    FLAGS((1 << 1) | (1 << 2)) ; /* Write + Read */
    bss     PT_LOAD    FLAGS((1 << 1) | (1 << 2)) ; /* Write + Read */
}

SECTIONS {
  . = 1M;

  .rodata :
  {
    *(.rodata .rodata.*)
    . = ALIGN(4K);
  } :rodata

  .text :
  {
    *(.text .text.*)
    . = ALIGN(4K);
  } :text

  .data :
  {
    *(.data .data.*)
    . = ALIGN(4K);
  } :data

  .bss :
  {
    *(.bss .bss.*)
    . = ALIGN(4K);
  } :bss
}
