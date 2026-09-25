use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=src/inline/ime_observer/dll.rs");
    println!("cargo:rerun-if-changed=src/inline/ime_observer/tsf.rs");
    println!("cargo:rerun-if-changed=src/inline/ime_observer/protocol.rs");
    println!("cargo:rerun-if-changed=src/caret/ffi.rs");
    println!("cargo:rerun-if-changed=src/caret/protocol.rs");
    println!("cargo:rerun-if-changed=src/caret/target_scheduler.rs");
    println!("cargo:rerun-if-changed=src/caret/tsf_abi.rs");
    println!("cargo:rerun-if-changed=src/caret/tsf_geometry.rs");
    println!("cargo:rerun-if-changed=src/caret/sensitivity.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    // The standalone DLL is compiled outside Cargo's crate graph. Forward
    // only the explicit native-test feature so fixture-only fault adapters
    // cannot enter ordinary Release/shadow/primary production binaries.
    let mut args = vec![
        "--crate-type".to_owned(),
        "cdylib".to_owned(),
        "--crate-name".to_owned(),
        "echo_ime_observer".to_owned(),
        "--edition".to_owned(),
        "2021".to_owned(),
        "--target".to_owned(),
        env::var("TARGET").expect("Cargo TARGET"),
        "-C".to_owned(),
        "opt-level=s".to_owned(),
        "-C".to_owned(),
        "panic=abort".to_owned(),
        "-C".to_owned(),
        "debuginfo=0".to_owned(),
    ];
    if env::var_os("CARGO_FEATURE_NATIVE_TEST").is_some() {
        args.push("--cfg".to_owned());
        args.push(r#"feature="native-test""#.to_owned());
    }
    let status = Command::new(env::var_os("RUSTC").expect("Cargo RUSTC"))
        .args(args)
        .arg("src/inline/ime_observer/dll.rs")
        .arg("--out-dir")
        .arg(output)
        .status()
        .expect("Build target-thread IME observer");
    assert!(status.success(), "Target-thread IME observer build failed");
}
