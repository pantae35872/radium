fn main() {
    println!("cargo:rustc-link-arg=-t");
    println!("cargo:rustc-link-arg=./src/userland/linker.ld");
}
