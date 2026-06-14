//! ADR 0191 — Rust-Lua Bridge MVP runtime.
//!
//! Compiled to a free-standing object by `build.rs` and linked
//! into every lumelir-produced binary via
//! `src/codegen/link.rs`. The `#![no_std]` keeps the object
//! dependency-free (no `eh_personality` / `panic_handler`
//! collisions when linked with `cc`).
//!
//! Extension protocol: add new `#[unsafe(no_mangle)] pub
//! extern "C" fn rust_<name>(...) -> ...` here, then add a
//! matching `Builtin::Rust*` variant (`src/hir/ir.rs`) and one
//! `rust_from_method` arm.

#![no_std]

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // The MVP bridge functions (currently just `rust_add`) are
    // leaf calls with no panic paths. The handler is included
    // only to satisfy the `#![no_std]` requirement.
    loop {}
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_add(a: f64, b: f64) -> f64 {
    a + b
}
