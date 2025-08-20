shell make dbg-run &
file build/kernel.bin
target remote :1234
break main.rs:48
skip function external_interrupt_handler
set disassembly-flavor intel
shell sleep 0.5
tu e
continue

define hook-quit
    shell kill -2 $(cat /tmp/dbg_make_pid.txt)
end
