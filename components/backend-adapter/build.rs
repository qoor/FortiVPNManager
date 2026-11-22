use std::env;
use std::path::Path;
use std::process::{Command, Stdio};

const MSYS2: &str = r"C:\msys64";

fn main() {
    println!("cargo:rerun-if-changed=src/openconnect/progress.c");
    cc::Build::new()
        .file("src/openconnect/progress.c")
        .compile("fvm_oc_progress");
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    if os != "macos" && os != "windows" {
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
    let mut build = if cfg!(windows) {
        let mut bash = Command::new(format!(r"{MSYS2}\usr\bin\bash.exe"));
        bash.arg(script.to_str().unwrap().replace('\\', "/"))
            .env("MSYSTEM", "UCRT64")
            .env("PATH", format!(r"{MSYS2}\ucrt64\bin;{MSYS2}\usr\bin"));
        bash
    } else {
        Command::new(&script)
    };
    let output = build.arg(&os).stderr(Stdio::inherit()).output().unwrap();
    assert!(output.status.success(), "{} {os} failed", script.display());
    let zip = String::from_utf8(output.stdout).unwrap();
    let zip = zip.trim();
    let out_dir = env::var("OUT_DIR").unwrap();
    let mut unzip = if cfg!(windows) {
        Command::new(format!(r"{MSYS2}\usr\bin\unzip.exe"))
    } else {
        Command::new("unzip")
    };
    unzip.args(["-q", "-o", zip]);
    if os == "macos" {
        let status = unzip.args(["lib/*", "-d", &out_dir]).status().unwrap();
        assert!(status.success(), "unzip {zip} failed");
        println!("cargo:rustc-link-search=native={out_dir}/lib");
        for library in ["openconnect", "ssl", "crypto"] {
            println!("cargo:rustc-link-lib=static={library}");
        }
        for library in ["xml2", "z", "iconv"] {
            println!("cargo:rustc-link-lib=dylib={library}");
        }
    } else {
        let bin = format!("{out_dir}/bin");
        let status = unzip.args(["-d", &bin]).status().unwrap();
        assert!(status.success(), "unzip {zip} failed");
        println!("cargo:rustc-link-search=native={bin}");
    }
}
