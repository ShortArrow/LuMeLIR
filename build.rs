//! ADR 0191 — Build-time compile of `src/bridge_runtime.rs` to a
//! free-standing object that lumelir-produced binaries link
//! against. The path is exposed to the compiler crate via
//! `cargo:rustc-env=LUMELIR_BRIDGE_OBJ=<path>`, picked up by
//! `src/codegen/link.rs` at compile time via `option_env!`.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by cargo"));
    let obj_path = out_dir.join("bridge_runtime.o");
    let src_path = PathBuf::from("src/bridge_runtime.rs");

    println!("cargo:rerun-if-changed=src/bridge_runtime.rs");
    println!("cargo:rerun-if-changed=build.rs");

    let status = Command::new("rustc")
        .args([
            "--edition",
            "2024",
            "--crate-type",
            "staticlib",
            "--emit",
            "obj",
            "-C",
            "panic=abort",
            "-C",
            "opt-level=2",
            "-C",
            "relocation-model=pic",
            "-o",
        ])
        .arg(&obj_path)
        .arg(&src_path)
        .status()
        .expect("failed to spawn rustc for bridge_runtime");
    assert!(
        status.success(),
        "rustc bridge_runtime build failed (status: {status:?})"
    );

    println!("cargo:rustc-env=LUMELIR_BRIDGE_OBJ={}", obj_path.display());

    // ADR 0315 — N5-A: coroutine core over <ucontext.h>, compiled
    // with the host cc (already a hard runtime dependency of
    // link.rs) and linked into generated binaries like the bridge.
    let coro_obj = out_dir.join("runtime_coroutine.o");
    let coro_src = PathBuf::from("src/runtime/coroutine.c");
    println!("cargo:rerun-if-changed=src/runtime/coroutine.c");
    let status = Command::new("cc")
        .args(["-c", "-O2", "-fPIC", "-o"])
        .arg(&coro_obj)
        .arg(&coro_src)
        .status()
        .expect("failed to spawn cc for runtime/coroutine.c");
    assert!(
        status.success(),
        "cc runtime/coroutine.c build failed (status: {status:?})"
    );
    println!("cargo:rustc-env=LUMELIR_CORO_OBJ={}", coro_obj.display());
}
