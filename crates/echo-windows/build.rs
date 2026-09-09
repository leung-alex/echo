use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=src/inline/ime_observer/dll.rs");
    println!("cargo:rerun-if-changed=src/inline/ime_observer/tsf.rs");
    println!("cargo:rerun-if-changed=src/inline/ime_observer/protocol.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let status = Command::new(env::var_os("RUSTC").expect("Cargo RUSTC"))
        .args([
            "--crate-type",
            "cdylib",
            "--crate-name",
            "echo_ime_observer",
            "--edition",
            "2021",
        ])
        .args(["--target", &env::var("TARGET").expect("Cargo TARGET")])
        .args([
            "-C",
            "opt-level=s",
            "-C",
            "panic=abort",
            "-C",
            "debuginfo=0",
        ])
        .arg("src/inline/ime_observer/dll.rs")
        .arg("--out-dir")
        .arg(output)
        .status()
        .expect("Build target-thread IME observer");
    assert!(status.success(), "Target-thread IME observer build failed");
}
