# 0191. Rust-Lua Bridge — MVP (`rust.add(a, b)` end-to-end)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-15
- **Deciders:** ShortArrow

## Context

[ADR 0189](0189-phase3-entry-criteria.md) §Phase 3 close criteria §1 requires "Rust-Lua Bridge entry ADR exists AND a minimum-viable Rust function is callable from Lua and verified end-to-end". This ADR is that entry + the MVP implementation.

The PRD memo identifies the Bridge as the kimo: "普段書いている Rust プロジェクトに、オーバーヘッドほぼゼロで Lua の柔軟性を持ち込めるようになる". The MVP must prove the concept end-to-end (Lua source → compiled native binary that calls a real `extern "C" fn` defined in Rust source), not just paper-design the protocol.

## Scope (literal)

- ✅ **`rust.<method>` namespace dispatch** at HIR — mirror of ADR 0103's `string.<method>` chokepoint via `Builtin::from_namespace_method`.
- ✅ **One demo function `rust.add(a: Number, b: Number) -> Number`** with `extern "C" fn rust_add(f64, f64) -> f64` semantics.
- ✅ **`build.rs`** that compiles `src/bridge_runtime.rs` to an object at build time via `rustc --emit=obj`, exposing the absolute path via a `cargo:rustc-env` variable.
- ✅ **Linker integration** at `src/codegen/link.rs::link_object` — pass the bridge object to `cc` so the compiled-Lua binary resolves `rust_add` at link time.
- ✅ **Extern declaration** in module init — `llvm.func @rust_add(f64, f64) -> f64` registered alongside `printf` / `strlen` / `sin` / etc.
- ✅ **3 e2e tests** in a new `tests/phase3_rust_lua_bridge_mvp.rs`: basic call (`print(rust.add(2.5, 3.5))` → `6.0`); shadowing positive (`local rust = {}; function rust.add(x, y) ... end` overrides the bridge); arity pin (`rust.add()` → `ArityMismatch`).
- ❌ **Non-Number signatures** — `String` / `Table` / `TaggedValue` args/returns. Deferred to a future Bridge-marshaling ADR (TBD). (Original plan reserved ADR 0192 for this; that slot was reassigned to Embedded register-ops entry. See [`docs/notes/leftover-roadmap.md` §F](../notes/leftover-roadmap.md).)
- ❌ **Variadic Rust signatures**. Deferred (rare in practice; not needed for kimo demo).
- ❌ **Error propagation** — Rust panic → Lua error. Deferred to a future Bridge-error-propagation ADR (TBD; pairs with pcall epilogue when triggered). (Original plan reserved ADR 0193; reassigned to Phase 4 entry criteria.)
- ❌ **GC-managed Lua values passed to Rust** — Bridge MVP uses primitive `f64` so no rooting / pin pressure. Deferred to a future Bridge-GC-interaction ADR (TBD; pairs with GC stack walk when actual freeing arrives). (Original plan reserved ADR 0194; reassigned to benchmark harness MVP.)
- ❌ **Multiple bridge functions** — one demo function only. Future bridge functions extend `src/bridge_runtime.rs` + the `rust_from_method` table; no architectural change required.
- ❌ **Dynamic linking** — bundled static object via `build.rs`. Sufficient for the MVP and the embedded-target story.
- ❌ **User-defined bridge crates** — out of scope; the bundled `bridge_runtime.rs` is the MVP surface. Future ADR opens up "drop-in bridge crate" extensibility.

## Decision

### HIR — `Builtin::RustAdd` + namespace dispatch

In `src/hir/ir.rs`:

```rust
pub enum Builtin {
    // ...existing variants...
    /// ADR 0191 — Rust-Lua Bridge MVP demo function.
    /// `rust.add(a: Number, b: Number) -> Number` dispatches to
    /// `extern "C" fn rust_add(f64, f64) -> f64` in
    /// `src/bridge_runtime.rs`.
    RustAdd,
}
```

`Builtin::rust_from_method`:

```rust
pub fn rust_from_method(method: &str) -> Option<Self> {
    match method {
        "add" => Some(Builtin::RustAdd),
        _ => None,
    }
}
```

`from_namespace_method` extension:

```rust
pub fn from_namespace_method(ns: &str, method: &str) -> Option<Self> {
    match ns {
        // ...existing arms...
        "rust" => Self::rust_from_method(method),
        _ => None,
    }
}
```

Per-variant metadata (`name`, `arity`, `ret_kinds`, `param_kinds_for_arity`):

- `name = "rust.add"`
- `arity = (2, 2)`
- `ret_kinds = &[ValueKind::Number]`
- `param_kinds_for_arity(2, i)` returns `Some(ValueKind::Number)` for `i < 2`, `None` otherwise.

`infer_kind` arm: `Callee::Builtin(Builtin::RustAdd) => ValueKind::Number`.

### Codegen — extern declaration + emit arm

In `src/codegen/emit.rs::emit_module_init` (or alongside `emit_libm_decls`), register:

```rust
let ty = llvm::r#type::function(types.f64, &[types.f64, types.f64], false);
LLVMFuncOperationBuilder::new(context, loc)
    .body(Region::new())
    .sym_name(StringAttribute::new(context, "rust_add"))
    .function_type(TypeAttribute::new(ty))
    .linkage(llvm::attributes::linkage(context,
        llvm::attributes::Linkage::External))
    .build();
```

In `Callee::Builtin(Builtin::RustAdd)` emit arm:

```rust
let arg_a = emit_expr(context, block, &args[0], ..., loc)?;
let arg_b = emit_expr(context, block, &args[1], ..., loc)?;
emit_libc_call_f64(context, block, "rust_add", &[arg_a, arg_b], types, loc)
```

(`emit_libc_call_f64` is the existing helper used by libm callers.)

### Bridge runtime — `src/bridge_runtime.rs`

New file containing the demo function:

```rust
//! ADR 0191 — Rust-Lua Bridge MVP runtime. Compiled to a free-
//! standing object by `build.rs` and linked into every
//! lumelir-produced binary via `src/codegen/link.rs`. Add new
//! `extern "C"` functions here + a matching `Builtin::Rust*`
//! variant + a `rust_from_method` arm to extend the bridge surface.

#![no_std]

#[unsafe(no_mangle)]
pub extern "C" fn rust_add(a: f64, b: f64) -> f64 {
    a + b
}
```

`#![no_std]` keeps the object dependency-free (no `eh_personality` / `panic_handler` collisions when linking with `cc`).

### `build.rs`

```rust
use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let obj_path = out_dir.join("bridge_runtime.o");
    let src_path = PathBuf::from("src/bridge_runtime.rs");

    println!("cargo:rerun-if-changed=src/bridge_runtime.rs");
    println!("cargo:rerun-if-changed=build.rs");

    let status = Command::new("rustc")
        .args([
            "--edition", "2024",
            "--crate-type", "staticlib",
            "--emit", "obj",
            "-C", "panic=abort",
            "-C", "opt-level=2",
            "-o",
        ])
        .arg(&obj_path)
        .arg(&src_path)
        .status()
        .expect("rustc bridge_runtime");
    assert!(status.success(), "bridge_runtime rustc failed");

    println!(
        "cargo:rustc-env=LUMELIR_BRIDGE_OBJ={}",
        obj_path.display()
    );
}
```

### `src/codegen/link.rs::link_object`

Extend to append the bridge object path when the env var is set at build time:

```rust
fn link_object(obj_path: &Path, output: &Path) -> Result<(), CodegenError> {
    let mut cmd = Command::new("cc");
    cmd.arg(obj_path).args(["-o"]).arg(output);
    if let Some(bridge_obj) = option_env!("LUMELIR_BRIDGE_OBJ") {
        cmd.arg(bridge_obj);
    }
    cmd.args(["-lc", "-lm"]);
    let status = cmd
        .status()
        .map_err(|e| CodegenError::LinkFailed(format!("failed to spawn cc: {e}")))?;
    if !status.success() {
        return Err(CodegenError::LinkFailed("cc linking failed".into()));
    }
    Ok(())
}
```

`option_env!` resolves at compile time so no runtime overhead beyond an `if let`.

### Tests

`tests/phase3_rust_lua_bridge_mvp.rs` (NEW):

1. **Basic call**: `print(rust.add(2.5, 3.5))` → `6.0`.
2. **Shadowing positive pin**: `local rust = {}; function rust.add(x, y) return x*y end; print(rust.add(3, 4))` → `12.0` (user table wins per ADR 0103 shadowing precedent).
3. **Arity pin**: `rust.add()` parse OK, HIR lower fails with `ArityMismatch`.

## Alternatives considered

- **Skip the namespace dispatch; treat `rust_add` as a top-level builtin like `print`.** Rejected — pollutes the global namespace, conflicts with user variable names, breaks Lua's principle of least surprise. Namespace-scoped (`rust.<method>`) is symmetric with `math` / `string` / `table` / `io`.
- **Dynamic loading of a Rust shared library at runtime.** Rejected for MVP — adds dlopen / dlsym surface and complicates static binary distribution. Static-link bundled bridge is the kimo demonstration; dynamic loading is a future extension.
- **Bridge crate as a separate Cargo workspace member.** Rejected — overkill for one demo function. `src/bridge_runtime.rs` + `build.rs` is the smallest viable shape; future ADR promotes to a workspace member when the bridge surface justifies it.
- **Use a `bindgen`-style approach** to auto-generate bridge declarations from Rust source. Rejected — the kimo claim is "オーバーヘッドほぼゼロ" inline call, not "auto-bindgen DX". Explicit per-function variant is the correct cost profile.
- **Defer the e2e test** because of build.rs + rustc-tool requirement. Rejected — Phase 3 close criterion §1 literally requires "verified end-to-end". No e2e, no Phase 3 close.

## Consequences

**Positive**
- Phase 3 close criterion §1 satisfied — Bridge ADR exists + MVP function callable end-to-end.
- PRD kimo claim has a concrete proof point.
- Extension surface is small: new bridge functions add one `Builtin::Rust*` variant + one line in `rust_from_method` + one `extern "C" fn` in `bridge_runtime.rs`.
- `option_env!` keeps the runtime overhead zero when the env var is absent (e.g. external rebuilds of `link.rs`).

**Negative**
- `build.rs` adds a build step that requires `rustc` on `PATH` (always true for cargo users).
- Bridge runtime is `#![no_std]`; future bridge functions needing `std` would require a separate strategy.
- Bundled-only — no extensibility for user-defined bridge crates without another ADR.

**Locked in until superseded**
- `rust.<method>` namespace is the SoT for Bridge dispatch. Top-level pollution is rejected.
- `extern "C"` calling convention is the cross-boundary ABI. Rust-mangled symbols are not used.
- `src/bridge_runtime.rs` + `build.rs` is the MVP shape. Larger Bridge surfaces refactor into a workspace member at the point of refactor, not preemptively.

## Documentation updates

- [x] §8 — adds 0191.
- [x] ADR 0189 §1 — close criterion satisfied; cross-reference here.
- [x] AGENTS row — new `3.0-rust-lua-bridge-mvp` lane.

## Test count delta

```
Step 0: 1427 (after f801d48)
C1 (doc): 1427 → 1427
C2 (3 Red Day 0 e2e): 1427 → 1427 (Red — `rust.<method>` not yet recognized)
C3 (impl): 1427 → 1430 (Green)
```

## Critical files

- `docs/design/0191-rust-lua-bridge-mvp.md` (this doc).
- `docs/design/README.md` index entry.
- `src/hir/ir.rs` — `Builtin::RustAdd` + `rust_from_method` + `from_namespace_method` extension.
- `src/hir/mod.rs` — `infer_kind` arm.
- `src/codegen/emit.rs` — extern declaration + emit arm.
- `src/codegen/link.rs` — bridge object link integration.
- `src/bridge_runtime.rs` (NEW).
- `build.rs` (NEW).
- `tests/phase3_rust_lua_bridge_mvp.rs` (NEW) — 3 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| `rustc` not on PATH at build time | Hard error from `build.rs`. Cargo users always have `rustc` available. |
| Bridge object ABI mismatch (e.g. `f64` parameter passing) | `extern "C"` is the C ABI, well-defined for `f64` on every target lumelir targets. |
| Bridge runtime `#![no_std]` conflict if a future function needs heap | Documented; future ADR adds a `std` variant or split-runtime. |
| `option_env!` returning None when env var should be set | Cargo guarantees env vars persist across compilation. Failure mode is graceful (bridge functions undefined symbol error at compile-Lua link time, surfaces clearly). |
| `cc` link order picks up wrong `rust_add` (e.g. from libc-extension) | Symbol is namespaced enough (`rust_add` is unlikely to collide); future bridge functions use `rust_` prefix uniformly. |
| `panic=abort` strips unwinding which conflicts with cc-linked main | Bridge functions are leaf calls with no panic paths in the MVP. Documented constraint. |
| Concurrent build artifacts (release vs debug) | `OUT_DIR` is profile-scoped by cargo. No cross-talk. |

## Future work

- ADR (TBD, post-Phase-4a) — Type marshaling (String / Table / Bool / TaggedValue across the boundary). The original 0192 reservation was reassigned to Embedded register-ops entry; this work now takes whichever number is next-available when scheduled.
- ADR (TBD, paired with pcall) — Error propagation (Rust panic → Lua error). Original 0193 reassigned to Phase 4 entry.
- ADR (TBD, paired with GC stack walk) — GC interaction (Rust holding Lua references). Original 0194 reassigned to benchmark harness MVP.
- ADR (TBD) — User-defined bridge crates (drop-in extensibility).
- ADR (TBD) — Embedded register-ops dialect entry (Phase 3 close criterion §2).

## References

- [ADR 0103](0103-stdlib-string-begin.md) — namespace builtin dispatch pattern reused here.
- [ADR 0189](0189-phase3-entry-criteria.md) — Phase 3 entry; this ADR satisfies §1.
- [ADR 0190](0190-phase2-epilogue-close-decisions.md) — Phase 2 epilogue deferrals; explains why Bridge MVP can ship without `pcall` / GC stack walk.
- PRD `docs/PRD.jp.md` §6 (Phase 3) and § "メモ" (Bridge as kimo).
- The Rustonomicon §`extern "C"` ABI.
