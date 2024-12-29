# Radium

Radium OS is currently in the very early stages of development and is not yet usable as a complete operating system.

## Build Dependices
* ```rust``` (lastest nightly)
* ```GNU Binutils``` (ld)
* ```nasm``` 
* ```xorriso```
* ```GNU mtools```
* ```dosfstools``` (mkfs.vfat)
* ```qemu``` (optional: require if you want to test or run it in qemu)
* ```wget```
* ```make```
## Getting the source code
```bash
git clone https://github.com/pantae35872/radium
cd radium
```
## Build and Run
Before building the OS, you might want to choose a font. It is recommended to use default font.
If you want to use custom font follow these step. 
1. Copy your font file (`.ttf`) to the project directory and 
2. rename it to ```kernel-font.ttf```
```bash
# if you use custom font skip this step Only Once!!
make font

# Create a virtual disk image. Only Once!!
make disk

# get OVMF.fd before running. Only Once!!
make ovmf

# build the os (release mode. if you want debug mode change the make argument to "debug")
make release

# run using QEMU
make run
```
## Contributing
At this time, I am not planning to accept contributions to the project.
