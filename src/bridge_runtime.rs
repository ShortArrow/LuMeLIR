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

// ADR 0224 — String → Number marshaling. `s_ptr` is the user-
// visible pointer of a Lua boxed-string object whose first 8
// bytes (offset 0) hold the i64 byte length (ADR 0112 layout).
// The function reads the length and returns it as f64 (Lua
// Number ABI).
// ADR 0225 — Bool ↔ Bool marshaling demo. Logical NOT over a
// single bool. C ABI passes `bool` as a 1-byte value (i8 in
// LLVM IR), but Lua's Bool slots are `i1`; the codegen extern
// declaration is shaped to match — see `rust_not_ty` in
// `src/codegen/emit.rs`.
#[unsafe(no_mangle)]
pub extern "C" fn rust_not(b: bool) -> bool {
    !b
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_strlen(s_ptr: *const u8) -> f64 {
    // SAFETY: the caller passes a Lua boxed-string-object ptr
    // whose layout begins with an aligned i64 length. The HIR
    // arg-kind validation guarantees only String values reach
    // this site.
    let len_i64 = unsafe { core::ptr::read_unaligned(s_ptr.cast::<i64>()) };
    len_i64 as f64
}
