fn main() {
    napi_build::setup();
    if std::env::var("TARGET").as_deref() == Ok("aarch64-apple-darwin") {
        // Use chained fixups so dyld can load the stripped arm64 binding.
        println!("cargo:rustc-link-arg-cdylib=-Wl,-fixup_chains");
    }
}
