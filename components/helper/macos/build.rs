fn main() {
    println!("cargo:rerun-if-changed=src/protocols.m");
    println!("cargo:rerun-if-changed=../../contract/macos/xpc/HelperXPC.h");
    cc::Build::new()
        .file("src/protocols.m")
        .include("../../contract/macos/xpc")
        .warnings_into_errors(true)
        .cargo_metadata(false)
        .compile("fvm_helper_protocols");
    println!(
        "cargo:rustc-link-search=native={}",
        std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")
    );
    println!("cargo:rustc-link-lib=static:+whole-archive=fvm_helper_protocols");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rerun-if-env-changed=FVM_INFO_PLIST");
    if let Ok(plist) = std::env::var("FVM_INFO_PLIST") {
        println!("cargo:rerun-if-changed={plist}");
        println!("cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{plist}");
    }
}
