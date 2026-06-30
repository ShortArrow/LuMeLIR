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

// ADR 0243 — M12-A: Bridge error propagation. The lumelir
// codegen exports a `lumelir_raise_error(msg_ptr)` wrapper (with
// External linkage) that stashes msg into g_error_value and
// longjmps. Routing through that helper keeps the bridge object
// free of direct symbol references to the lumelir globals (which
// have Internal linkage and aren't visible at link time).
unsafe extern "C" {
    fn lumelir_raise_error(msg_ptr: *const u8) -> !;
}

/// ADR 0243 — `rust.fail(msg)` raises a Lua error from Rust.
/// `msg_ptr` is the user-visible boxed-string-object ptr (ADR
/// 0112 layout). The function delegates to the codegen-emitted
/// `lumelir_raise_error` which stashes the value and longjmps
/// to the nearest landing pad. Inside `pcall`, the caught path
/// returns `(false, msg)`; otherwise the chunk-level pad prints
/// the message and exits 1.
#[unsafe(no_mangle)]
pub extern "C" fn rust_fail(msg_ptr: *const u8) -> ! {
    unsafe {
        lumelir_raise_error(msg_ptr);
    }
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

// ADR 0231 — M7-D: pattern matcher with character classes
// (`%a` / `%d` / `%s` / `%w` / `%p` / `%l` / `%u` / `%x` / `%c`
// and their complements) + `.` wildcard + `%X` magic-char
// escape. No quantifiers / sets / anchors / captures yet.
// Returns a packed i64: low 32 bits = 1-indexed start, high 32
// bits = matched byte length. 0 = no match.
fn matches_class(c: u8, class: u8) -> bool {
    match class {
        b'a' => c.is_ascii_alphabetic(),
        b'A' => !c.is_ascii_alphabetic(),
        b'd' => c.is_ascii_digit(),
        b'D' => !c.is_ascii_digit(),
        b's' => c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' || c == 0x0b || c == 0x0c,
        b'S' => !(c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' || c == 0x0b || c == 0x0c),
        b'w' => c.is_ascii_alphanumeric(),
        b'W' => !c.is_ascii_alphanumeric(),
        b'p' => c.is_ascii_punctuation(),
        b'P' => !c.is_ascii_punctuation(),
        b'l' => c.is_ascii_lowercase(),
        b'L' => !c.is_ascii_lowercase(),
        b'u' => c.is_ascii_uppercase(),
        b'U' => !c.is_ascii_uppercase(),
        b'x' => c.is_ascii_hexdigit(),
        b'X' => !c.is_ascii_hexdigit(),
        b'c' => c.is_ascii_control(),
        b'C' => !c.is_ascii_control(),
        // `%.` → literal `.`, `%%` → literal `%`, etc.
        _ => c == class,
    }
}

// All slice accesses below use `get_unchecked` because the
// `#![no_std]` bridge object has no `core::panicking::*`
// resolution; each call site explicitly bounds-checks first.

#[inline]
unsafe fn at(s: &[u8], i: usize) -> u8 {
    unsafe { *s.get_unchecked(i) }
}

// ADR 0282 — N4-E: set body match. `pat[p_i]` is `[`. Walks
// the body until `]`, testing each element (literal char, `%X`
// class, or `a-z` range). Leading `^` negates.
unsafe fn matches_set(c: u8, pat: &[u8], p_i: usize) -> bool {
    let mut j = p_i + 1;
    let negate = j < pat.len() && unsafe { at(pat, j) } == b'^';
    if negate {
        j += 1;
    }
    let mut matched = false;
    while j < pat.len() && unsafe { at(pat, j) } != b']' {
        let b = unsafe { at(pat, j) };
        if b == b'%' && j + 1 < pat.len() {
            if matches_class(c, unsafe { at(pat, j + 1) }) {
                matched = true;
            }
            j += 2;
            continue;
        }
        // Range `b-X` (X must not be `]`).
        if j + 2 < pat.len()
            && unsafe { at(pat, j + 1) } == b'-'
            && unsafe { at(pat, j + 2) } != b']'
        {
            let lo = b;
            let hi = unsafe { at(pat, j + 2) };
            if c >= lo && c <= hi {
                matched = true;
            }
            j += 3;
            continue;
        }
        if c == b {
            matched = true;
        }
        j += 1;
    }
    matched != negate
}

// ADR 0231 — single-byte atom test: literal, `.`, `%X` class,
// or ADR 0282 `[...]` set.
// SAFETY: caller guarantees p_i < pat.len(); for `%X` atoms,
// caller also guarantees p_i + 1 < pat.len() (via atom_len).
unsafe fn matches_atom(c: u8, pat: &[u8], p_i: usize) -> bool {
    let p = unsafe { at(pat, p_i) };
    if p == b'%' {
        matches_class(c, unsafe { at(pat, p_i + 1) })
    } else if p == b'.' {
        true
    } else if p == b'[' {
        unsafe { matches_set(c, pat, p_i) }
    } else {
        c == p
    }
}

// Returns the byte length of a single pattern atom (1 for
// literal / `.`, 2 for `%X`, variable for `[...]`).
// SAFETY: caller guarantees p_i < pat.len().
unsafe fn atom_len(pat: &[u8], p_i: usize) -> usize {
    let p = unsafe { at(pat, p_i) };
    if p == b'%' {
        return 2;
    }
    if p != b'[' {
        return 1;
    }
    // ADR 0282 — span to closing `]`. `%X` inside the set
    // consumes two bytes so a literal `%]` doesn't close.
    let mut j = p_i + 1;
    if j < pat.len() && unsafe { at(pat, j) } == b'^' {
        j += 1;
    }
    while j < pat.len() {
        let b = unsafe { at(pat, j) };
        if b == b'%' && j + 1 < pat.len() {
            j += 2;
            continue;
        }
        if b == b']' {
            return j - p_i + 1;
        }
        j += 1;
    }
    // Malformed — treat as rest of pattern (match_pattern's
    // bounds check will then refuse to advance).
    pat.len() - p_i
}

// ADR 0281 — N4-D: single non-nested capture state threaded
// through `match_pattern`. Snapshot-restored on every failed try.
#[derive(Copy, Clone, Default)]
struct CapState {
    has_pending: bool,
    pending_start: usize,
    captured: bool,
    cap_start: usize,
    cap_end: usize,
}

// ADR 0279 — N4-B: recursive backtracking match with quantifiers
// `*` / `+` / `?`. Returns Some(s_end_index) on success.
// ADR 0280 — N4-C: `$` at the final pattern position anchors to
// end-of-string.
// ADR 0281 — N4-D: `(` / `)` record/commit a single capture.
unsafe fn match_pattern(
    s: &[u8],
    s_i: usize,
    pat: &[u8],
    p_i: usize,
    cap: &mut CapState,
) -> Option<usize> {
    if p_i >= pat.len() {
        return Some(s_i);
    }
    // End-of-string anchor: `$` as the *last* pattern byte means
    // the match must consume up to s.len(). Anywhere else it's a
    // literal `$` (handled by the normal atom path).
    if p_i + 1 == pat.len() && unsafe { at(pat, p_i) } == b'$' {
        return if s_i == s.len() { Some(s_i) } else { None };
    }
    // ADR 0281 — capture open.
    if unsafe { at(pat, p_i) } == b'(' {
        if cap.has_pending {
            return None;
        }
        let saved = *cap;
        cap.has_pending = true;
        cap.pending_start = s_i;
        if let Some(end) = unsafe { match_pattern(s, s_i, pat, p_i + 1, cap) } {
            return Some(end);
        }
        *cap = saved;
        return None;
    }
    // ADR 0281 — capture close.
    if unsafe { at(pat, p_i) } == b')' {
        if !cap.has_pending {
            return None;
        }
        let saved = *cap;
        cap.has_pending = false;
        cap.captured = true;
        cap.cap_start = saved.pending_start;
        cap.cap_end = s_i;
        if let Some(end) = unsafe { match_pattern(s, s_i, pat, p_i + 1, cap) } {
            return Some(end);
        }
        *cap = saved;
        return None;
    }
    let alen = unsafe { atom_len(pat, p_i) };
    // Truncated escape — treat as no-match (defensive; ADR 0231
    // returned None on this shape too).
    if p_i + alen > pat.len() {
        return None;
    }
    let after_atom = p_i + alen;
    let quant = if after_atom < pat.len() {
        let q = unsafe { at(pat, after_atom) };
        if q == b'*' || q == b'+' || q == b'?' { Some(q) } else { None }
    } else {
        None
    };
    match quant {
        Some(b'*') => unsafe { match_max(s, s_i, pat, p_i, after_atom + 1, 0, cap) },
        Some(b'+') => unsafe { match_max(s, s_i, pat, p_i, after_atom + 1, 1, cap) },
        Some(b'?') => unsafe { match_opt(s, s_i, pat, p_i, after_atom + 1, cap) },
        _ => {
            if s_i >= s.len() {
                return None;
            }
            if unsafe { matches_atom(at(s, s_i), pat, p_i) } {
                unsafe { match_pattern(s, s_i + 1, pat, after_atom, cap) }
            } else {
                None
            }
        }
    }
}

unsafe fn match_max(
    s: &[u8],
    s_i: usize,
    pat: &[u8],
    atom_p: usize,
    rest_p: usize,
    min: usize,
    cap: &mut CapState,
) -> Option<usize> {
    let mut count = 0usize;
    while s_i + count < s.len() && unsafe { matches_atom(at(s, s_i + count), pat, atom_p) } {
        count += 1;
    }
    loop {
        if count >= min {
            let saved = *cap;
            if let Some(end) = unsafe { match_pattern(s, s_i + count, pat, rest_p, cap) } {
                return Some(end);
            }
            *cap = saved;
        }
        if count == 0 {
            return None;
        }
        count -= 1;
    }
}

unsafe fn match_opt(
    s: &[u8],
    s_i: usize,
    pat: &[u8],
    atom_p: usize,
    rest_p: usize,
    cap: &mut CapState,
) -> Option<usize> {
    if s_i < s.len() && unsafe { matches_atom(at(s, s_i), pat, atom_p) } {
        let saved = *cap;
        if let Some(end) = unsafe { match_pattern(s, s_i + 1, pat, rest_p, cap) } {
            return Some(end);
        }
        *cap = saved;
    }
    unsafe { match_pattern(s, s_i, pat, rest_p, cap) }
}

// Try to match `pat` starting at `s[s_start]`. Returns
// `Some((consumed_s_bytes, CapState))` on success.
fn try_match_at(s: &[u8], s_start: usize, pat: &[u8]) -> Option<(usize, CapState)> {
    let mut cap = CapState::default();
    let end = unsafe { match_pattern(s, s_start, pat, 0, &mut cap) }?;
    Some((end - s_start, cap))
}

// ADR 0281 — packed result of an overall pattern search:
// (start, len) where start is 1-indexed and 0 means no match.
// Captures live in the threaded CapState; the caller decides
// whether to expose them.
// ADR 0283 — N4-F-1: `init_byte` is the 0-indexed starting byte
// offset within `s`. Passing 0 reproduces the ADRs 0231 / 0281
// behaviour.
unsafe fn run_pattern_from(
    s_ptr: *const u8,
    pat_ptr: *const u8,
    init_byte: usize,
) -> Option<(usize, usize, CapState)> {
    unsafe {
        let s_len = core::ptr::read_unaligned(s_ptr.cast::<i64>()) as usize;
        let pat_len = core::ptr::read_unaligned(pat_ptr.cast::<i64>()) as usize;
        let s = core::slice::from_raw_parts(s_ptr.add(8), s_len);
        let pat = core::slice::from_raw_parts(pat_ptr.add(8), pat_len);
        if pat_len == 0 {
            return Some((init_byte.min(s_len), 0, CapState::default()));
        }
        let (anchored, pat) = if *pat.get_unchecked(0) == b'^' {
            (
                true,
                core::slice::from_raw_parts(pat.as_ptr().add(1), pat_len - 1),
            )
        } else {
            (false, pat)
        };
        if init_byte > s_len {
            return None;
        }
        let mut start = init_byte;
        let stop = if anchored { init_byte } else { s_len };
        loop {
            if let Some((consumed, cap)) = try_match_at(s, start, pat) {
                return Some((start, consumed, cap));
            }
            if start >= stop {
                return None;
            }
            start += 1;
        }
    }
}

unsafe fn run_pattern(s_ptr: *const u8, pat_ptr: *const u8) -> Option<(usize, usize, CapState)> {
    unsafe { run_pattern_from(s_ptr, pat_ptr, 0) }
}

#[unsafe(no_mangle)]
pub extern "C" fn lumelir_string_match_extents(s_ptr: *const u8, pat_ptr: *const u8) -> i64 {
    match unsafe { run_pattern(s_ptr, pat_ptr) } {
        Some((start, consumed, _cap)) => {
            let pos = (start as i64) + 1;
            let len = consumed as i64;
            (len << 32) | pos
        }
        None => 0,
    }
}

// ADR 0283 — N4-F-1: `string.find(s, pat, init)` 3-arg form.
// `init_1based` is the 1-indexed start position; passing 1
// reproduces the no-init behaviour. Returns the overall match
// extent (start, len) packed in i64; 0 = no match.
#[unsafe(no_mangle)]
pub extern "C" fn lumelir_string_find_init(
    s_ptr: *const u8,
    pat_ptr: *const u8,
    init_1based: i64,
) -> i64 {
    let init_byte = if init_1based <= 1 {
        0
    } else {
        (init_1based - 1) as usize
    };
    match unsafe { run_pattern_from(s_ptr, pat_ptr, init_byte) } {
        Some((start, consumed, _cap)) => {
            let pos = (start as i64) + 1;
            let len = consumed as i64;
            (len << 32) | pos
        }
        None => 0,
    }
}

// ADR 0281 — N4-D: like `lumelir_string_match_extents`, but if
// the pattern contained a capture, returns that capture's
// (pos, len) instead of the overall match. Returns the overall
// match extent when no capture is present (Lua spec for
// `string.match`).
#[unsafe(no_mangle)]
pub extern "C" fn lumelir_string_match_capture1(s_ptr: *const u8, pat_ptr: *const u8) -> i64 {
    match unsafe { run_pattern(s_ptr, pat_ptr) } {
        Some((start, consumed, cap)) => {
            if cap.captured {
                let pos = (cap.cap_start as i64) + 1;
                let len = (cap.cap_end - cap.cap_start) as i64;
                (len << 32) | pos
            } else {
                let pos = (start as i64) + 1;
                let len = consumed as i64;
                (len << 32) | pos
            }
        }
        None => 0,
    }
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
