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

// ADR 0228 — M7-A: `string.find(s, sub)` literal substring
// search runtime helper. Returns 1-indexed start position in
// `s`, or 0 if not found. Magic-char pattern semantics are
// deferred to a future M7 sub-ADR; this entry handles only the
// "plain" case. Lives in the bridge object file for the same
// link-time convenience as the bridge surface but is reached
// via `string.find` (not `rust.*`).
#[unsafe(no_mangle)]
pub extern "C" fn lumelir_string_find_plain(s_ptr: *const u8, sub_ptr: *const u8) -> i64 {
    unsafe {
        let s_len = core::ptr::read_unaligned(s_ptr.cast::<i64>()) as usize;
        let sub_len = core::ptr::read_unaligned(sub_ptr.cast::<i64>()) as usize;
        if sub_len == 0 {
            // Empty pattern matches at start (Lua semantics).
            return 1;
        }
        if sub_len > s_len {
            return 0;
        }
        let s_data = s_ptr.add(8);
        let sub_data = sub_ptr.add(8);
        let last = s_len - sub_len;
        let mut i = 0usize;
        while i <= last {
            let mut j = 0usize;
            while j < sub_len {
                if *s_data.add(i + j) != *sub_data.add(j) {
                    break;
                }
                j += 1;
            }
            if j == sub_len {
                return (i as i64) + 1;
            }
            i += 1;
        }
        0
    }
}

// ADR 0226 — mixed-kind multi-arg marshaling: String + String →
// Bool. Returns `true` when `s` begins with `prefix`. Reads both
// length headers (ADR 0112 layout) and compares the first
// `prefix_len` data bytes; short-circuits when prefix is longer
// than s.
#[unsafe(no_mangle)]
pub extern "C" fn rust_starts_with(s_ptr: *const u8, prefix_ptr: *const u8) -> bool {
    unsafe {
        let s_len = core::ptr::read_unaligned(s_ptr.cast::<i64>()) as usize;
        let p_len = core::ptr::read_unaligned(prefix_ptr.cast::<i64>()) as usize;
        if p_len > s_len {
            return false;
        }
        // Data lives at offset 8 in both buffers.
        let s_data = s_ptr.add(8);
        let p_data = prefix_ptr.add(8);
        let mut i = 0usize;
        while i < p_len {
            if *s_data.add(i) != *p_data.add(i) {
                return false;
            }
            i += 1;
        }
        true
    }
}
