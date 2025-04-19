use std::{env, path::Path, process::Command};

fn run(cmd: &mut Command, fail_msg: &str) {
    let status = cmd.status().expect(fail_msg);
    if !status.success() {
        panic!("{} (exit code: {})", fail_msg, status);
    }
}

fn main() {
    nasm_rs::compile_library_args("bootlib", &["src/boot/boot.asm"], &["-felf64"]).unwrap();
    let outdir = env::var_os("OUT_DIR").expect("Out dir must set");
    let path = Path::new(&outdir).join("trampoline.bin");
    let trampoline_o = Path::new(&outdir).join("trampoline.o");
    run(
        Command::new("fasm")
            .arg("src/boot/trampoline.asm")
            .arg(&path),
        "Failed to compile trampoline",
    );
    run(
        Command::new("ld")
            .args(["-r", "-b", "binary", "-o", "trampoline.o", "trampoline.bin"])
            .current_dir(&outdir),
        "Failed to convert flat binary to .o",
    );
    run(
        Command::new("objcopy")
            .args(["--rename-section", ".data=.trampoline_data"])
            .arg("trampoline.o")
            .current_dir(&outdir),
        "Failed to rename section for the trampoline object",
    );
    println!("cargo:rustc-link-arg={}/boot.o", outdir.display());
    println!("cargo:rustc-link-arg={}", trampoline_o.display());
    println!("cargo:rustc-link-arg=-T");
    println!("cargo:rustc-link-arg=linker.ld");
}
