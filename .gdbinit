shell make dbg-run &
file build/kernel.bin
target remote :1234
break start
shell sleep 0.5
tu e
continue

define hook-quit
    shell kill -2 $(cat /tmp/dbg_make_pid.txt)
end
