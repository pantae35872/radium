fn main() {
    println!("cargo:rustc-link-arg=-t");
    println!("cargo:rustc-link-arg=kmodule_linker.ld");
}
