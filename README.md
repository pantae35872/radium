# Radium
# THIS BRANCH EXISTS ONLY FOR REPRODUCING BUG: CURRENT BUG IS RUST COMPARE BYTES

## How to reproduce COMPARE BYTES BUG
First you need to build the project using make following the instruction below,
when you ran the project using make run, it'll boot up a qemu virtual machine.

Then you suppose to see the logs being flushed, and the last log will be the UD invalid opcode panic that causes by assert_unsafe_precondition in the compare bytes function.

The code that is causing the problem is in src/kernel/src/main.rs, you'll see the reproduction code there. you can remove the compare_bytes function at the end of the file, to prove to yourself that it is the problem.

I've setup gdb script for you it's in .gdbinit, add it to your gdb safe path, build the project then run gdb and it'll start a virtual machine for you. wait for a few seconds (depending on the machine). you may hit CTRL-L if the tui is not loading. 

I've setup a break point in compare_bytes_bug function, you can debug your way there.

![Project Status](https://img.shields.io/badge/status-not%20finished-orange)
![Build](https://img.shields.io/badge/build-passing-brightgreen)

Radium OS is currently in the very early stages of development and is no where near usable.

## Build Dependices
* ```rust``` (lastest nightly)
* ```GNU Binutils``` (ld)
* ```nasm``` 
* ```xorriso```
* ```GNU mtools```
* ```dosfstools``` (mkfs.vfat)
* ```qemu```
* ```wget```
* ```make```
* ```ovmf```
## Getting the source code
This project uses sub modules, YOU DON'T HAVE TO IN THIS BRANCH
```bash
git clone https://github.com/pantae35872/radium
cd radium
git checkout reproduce # Make sure to checkout the reproduce branch 
```
## Build and Run
Before building the OS, you might want to choose a font. It is recommended to use default font.
but if you want to use custom font follow these step. 
1. Copy your font file (`.ttf`) to the project directory and 
2. rename it to ```kernel-font.ttf```
```bash
# build the os (default is debug mode)
make

# run using QEMU
make run

# debug using gdb (the script is in .gdbinit file, add it to your gdb safe path)
gdb 
```
## Contributing
At this time, I am not planning to accept contributions to the project.
