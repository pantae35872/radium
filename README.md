# Radium

![Project Status](https://img.shields.io/badge/status-not%20finished-orange)
![Build](https://img.shields.io/badge/build-passing-brightgreen)

> [!WARNING]
> The build may not success on your system, because of edk2, and that's intentional because the project is not finished, you can remove the ovmf build in the Makefile and use your ovmf installation

Radium OS is currently in the very early stages of development and is no where near usable.

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
This project uses sub modules
```bash
git clone --recurse-submodules https://github.com/pantae35872/radium
cd radium
```
## Build and Run
Before building the OS, you might want to choose a font. It is recommended to use default font.
but if you want to use custom font follow these step. 
1. Copy your font file (`.ttf`) to the project directory and 
2. rename it to ```kernel-font.ttf```
```bash
# build the os (release mode. if you want debug mode change the make argument to "debug")
make release

# run using QEMU
make run
```
## Contributing
At this time, I am not planning to accept contributions to the project.
