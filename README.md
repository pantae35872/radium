# Nothing OS

Nothing OS is currently in the very early stages of development and is not yet usable as a complete operating system.

## Build Dependices
* ```rust``` (lastest nightly)
* ```GNU Binutils``` (ld)
* ```nasm``` 
* ```xorriso```
* ```GNU mtools```
* ```dosfstools``` (mkfs.vfat)
* ```qemu``` (optional: require if you want to test or run it in qemu)
## Build and Run
Before building the OS, you need to choose a font. It is recommended to use OpenSans Regular. 
Copy the font file (`.ttf`) to the project directory and rename it to ```kernel-font.ttf```
```bash
# release build
make release

# Create a virtual disk image (run this before running the OS for the first time)
make disk

# run using QEMU
make run
```
