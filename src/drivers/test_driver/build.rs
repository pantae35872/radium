fn main() {
    println!("cargo:rustc-link-arg=-t");
    println!("cargo:rustc-link-arg=kdriver_linker.ld");
}
