# Radium

![Project Status](https://img.shields.io/badge/status-not%20finished-orange)
![Build](https://img.shields.io/badge/build-passing-brightgreen)

> [!WARNING]
> The build may not success on your system, because of edk2, and that's intentional because the project is not finished, you can remove the ovmf build in the Makefile and use your ovmf installation

Radium OS is currently in the very early stages of development and is no where near usable.

## Build Dependices
* ```rust``` (lastest nightly)
* ```nasm``` 
* ```fasm```
* ```wget```
* ```qemu``` (optional: require if you want to test or run it in qemu)
## Getting the source code
This project uses sub modules
```bash
git clone --recurse-submodules https://github.com/pantae35872/radium
cd radium
```
## Build and Run
```bash
cargo run --release
```
Then type command `build` into the prompt, it'll build and run the project, for more information on the build system, type command `help`
## Contributing
At this time, I am not planning to accept contributions to the project.
