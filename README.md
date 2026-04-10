# Radium

![Project Status](https://img.shields.io/badge/status-not%20finished-orange)
![Build](https://img.shields.io/badge/build-passing-brightgreen)

Radium OS is currently in the very early stages of development and is no where near usable.

## Build Dependices
* ```rust``` (lastest nightly)
* ```fasm```
* ```wget```
* ```qemu``` (optional: require if you want to test or run it in qemu)
## Getting the source code
Do not do recursive submodule the build system will do it if required
```bash
git clone https://github.com/pantae35872/radium
cd radium
```
## Build and Run
#### Unix
```bash
cargo run --release
```
Then type command `build` into the prompt, it'll build and run the project, for more information on the build system, type command `help`.
If you don't want TUI set RADIUM_BUILD_TOOL_NO_TUI to anything (except false or 0) before running, followed by the command you want to run. e.g.
```bash
# No TUI mode example

RADIUM_BUILD_TOOL_NO_TUI=true cargo run --release -- build
# with qemu
RADIUM_BUILD_TOOL_NO_TUI=true cargo run --release -- build -qemu.run true
```
#### What about windows...
No just no (but wsl should work)
## Contributing
At this time, I am not planning to accept contributions to the project.
