use std::{
    env, fs,
    path::{Path, absolute},
    process::Command,
};

use uuid::Uuid;

fn run(cmd: &mut Command, fail_msg: &str) {
    let status = cmd.status().expect(fail_msg);
    if !status.success() {
        panic!("{} (exit code: {})", fail_msg, status);
    }
}

fn main() {
    println!("cargo:rerun-if-changed=../../build/config.rs");
    let outdir = env::var_os("OUT_DIR").expect("Out dir must set");
    fs::copy(absolute(Path::new("../../build/config.rs")).unwrap(), Path::new(&outdir).join("config.rs")).unwrap();

    println!("cargo:rerun-if-changed=./asm");

    let outdir = env::var_os("OUT_DIR").expect("Out dir must set");
    let boot_o = Path::new(&outdir).join("boot.o");
    run(Command::new("fasm").arg("asm/boot.asm").arg(&boot_o), "Failed to compile boot");

    let trampoline_bin = Path::new(&outdir).join("trampoline.bin");
    run(Command::new("fasm").arg("asm/trampoline.asm").arg(&trampoline_bin), "Failed to compile trampoline");

    let trampoline_o = Path::new(&outdir).join("trampoline.o");
    run(
        Command::new("ld").args(["-r", "-b", "binary", "-o"]).arg(&trampoline_o).arg(&trampoline_bin),
        "Failed to convert flat binary to .o",
    );
    run(
        Command::new("objcopy").args(["--rename-section", ".data=.trampoline_data"]).arg(&trampoline_o),
        "Failed to rename section for the trampoline object",
    );

    println!("cargo:rustc-link-arg={}", trampoline_o.display());
    println!("cargo:rustc-link-arg={}", boot_o.display());
    println!("cargo:rustc-link-arg=-T");
    println!("cargo:rustc-link-arg=linker.ld");
    println!("cargo:rustc-env=BUILD_UUID={}", Uuid::new_v4());
}
