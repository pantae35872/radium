shell setsid make dbg-run &
file build/kernel.bin
target remote :1234
break start
shell sleep 0.5
tu e
continue

define hook-quit
    shell kill -2 $(pgrep -o qemu-system-x86)
end
