file build/x86_64/debug/radium
target remote :1234
hbreak start
set disassembly-flavor intel
shell sleep 0.5
tu e
continue
