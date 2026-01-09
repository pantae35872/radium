use std::{
    env, fs,
    path::{Path, absolute},
};

fn main() {
    println!("cargo:rerun-if-changed=../../build/config.rs");
    let outdir = env::var_os("OUT_DIR").expect("Out dir must set");
    fs::copy(absolute(Path::new("../../build/config.rs")).unwrap(), Path::new(&outdir).join("config.rs")).unwrap();
}
