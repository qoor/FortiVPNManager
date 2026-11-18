use std::env;
use std::path::Path;
use std::process::{Command, Stdio};

fn main() {
    println!("cargo:rerun-if-changed=src/openconnect/progress.c");
    cc::Build::new()
        .file("src/openconnect/progress.c")
        .compile("fvm_oc_progress");
    if env::var("CARGO_CFG_TARGET_OS").unwrap() != "macos" {
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = root.join("packaging/openconnect/build.sh");
    let index = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "--path-format=absolute", "--git-path", "index"])
        .output()
        .unwrap();
    assert!(
        index.status.success(),
        "git rev-parse --git-path index failed"
    );
    println!("cargo:rerun-if-changed={}", script.display());
    println!(
        "cargo:rerun-if-changed={}",
        String::from_utf8(index.stdout).unwrap().trim()
    );
    let output = Command::new(&script)
        .arg("macos")
        .stderr(Stdio::inherit())
        .output()
        .unwrap();
    assert!(output.status.success(), "{} macos failed", script.display());
    let zip = String::from_utf8(output.stdout).unwrap();
    let out_dir = env::var("OUT_DIR").unwrap();
    let status = Command::new("unzip")
        .args(["-q", "-o", zip.trim(), "lib/*", "-d", &out_dir])
        .status()
        .unwrap();
    assert!(status.success(), "unzip {} failed", zip.trim());
    println!("cargo:rustc-link-search=native={out_dir}/lib");
    for library in ["openconnect", "ssl", "crypto"] {
        println!("cargo:rustc-link-lib=static={library}");
    }
    for library in ["xml2", "z", "iconv"] {
        println!("cargo:rustc-link-lib=dylib={library}");
    }
}
