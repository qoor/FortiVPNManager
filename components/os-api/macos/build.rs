fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    println!("cargo:rerun-if-changed=src/log.m");
    cc::Build::new()
        .file("src/log.m")
        .warnings_into_errors(true)
        .compile("fvm_os_api_log");
}
