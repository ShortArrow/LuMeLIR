# 0249. Unsafe-Block Audit — Phase 4 Production Hardening

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

First M15 sub-ADR. ADR 0193 §Workstreams listed "unsafe-block audit (ADR 0195 §Workstreams)" as a Phase 4 production-hardening item. This ADR enumerates every `unsafe` block in `src/`, categorises each by purpose, and confirms each has a SAFETY justification commensurate with the risk.

The audit is a docs-only deliverable: no code rewrites. Future ADRs can revisit specific sites if profiling or code-review surfaces concerns.

## Audit summary

**Total unsafe sites: 31** across two files (`src/bridge_runtime.rs`, `src/codegen/emit.rs`).

### Bridge runtime (6 sites)

`src/bridge_runtime.rs` is `#![no_std]` Rust compiled to a free-standing object and linked into every produced binary. All bridge functions touch raw `*const u8` pointers passed across the FFI boundary; `unsafe` is mandatory by Rust semantics.

| Site | Operation | SAFETY argument |
|---|---|---|
| `unsafe extern "C" { fn lumelir_raise_error... }` (ADR 0243) | extern decl | Lumelir codegen emits `lumelir_raise_error` with External linkage; symbol resolves at link time. |
| `rust_strlen`: `read_unaligned(s_ptr.cast::<i64>())` | reads i64 length header | HIR validation guarantees only String values reach `rust.strlen`; ADR 0112 layout pins the i64 header at offset 0. |
| `rust_starts_with`: same length-read pattern × 2 | reads two i64 headers | Same guarantees as `rust_strlen`; both args are HIR-validated Strings. |
| `lumelir_string_match_extents`: `slice::from_raw_parts(s_ptr.add(8), s_len)` | constructs slice over String payload | The boxed-string layout guarantees `s_ptr + 8` is the payload start and `s_len` from the header doesn't exceed the allocation (`GC_HEADER_OFF_SIZE` is the 4 GiB guard from ADR 0184). |
| `lumelir_string_find_plain`: `s_ptr.add(8)` / `sub_ptr.add(8)` | offsets into payload | Same boxed-string-layout guarantee. |
| `rust_fail`: `lumelir_raise_error(msg_ptr)` | calls the noreturn helper | The helper stashes `msg_ptr` + longjmps; this is a tail-call to an established external function. |

**Verdict**: all bridge unsafes are FFI-mandatory + ADR-justified. No simplification opportunity without trading the FFI for a slower marshaling layer.

### Codegen Value lifetime transmutes (24 sites)

`src/codegen/emit.rs` lines 3070, 3110, 3178, 3180, 3330, 3356, 3455, 3470, 3511, 3603, 3605, 5435, 5437, 5461, 5462, 5464, 17223, 21191, 21436, 21682, 21683, 21749, 21778.

All 24 are `std::mem::transmute::<Value<'c, 'a>, Value<'c, '_>>(...)` or the `&[Value]` variant. The pattern resolves a melior lifetime constraint: `Value<'c, 'block>` is invariant in `'block`, but melior's borrow checker can't see that an alloca'd value's address dominates inner scf.if / scf.while regions. Rust rejects the implicit conversion; the transmute teaches the compiler what melior's own type system would express if it could.

The `transmute_slots` helper (line 21431) has the canonical SAFETY comment:

> Value<'c, '_> is covariant in the second lifetime; the alloca'd pointers dominate every inner region, so referencing them from inner blocks is well-typed at the MLIR level. Rust's type system can't see this dominance relation, so we cast.

**Verdict**: all 24 sites share the same invariant (alloca-dominance over inner-region access). They are correct given the melior API's lifetime annotations. Future melior versions may relax the invariance, eliminating the need for the transmutes; until then they stay.

### `String::from_utf8_unchecked` (1 site)

`src/codegen/emit.rs:1087` — boxed-string-global initialiser. SAFETY comment:

> bytes pass verbatim to MLIR's string attribute and emit as `[N x i8] c"..."` in LLVM IR. Neither MLIR nor LLVM interprets them as UTF-8; the Rust `String` is just a transport carrier for the `&str` arg of `StringAttribute::new`.

The bytes can contain non-UTF-8 sequences (boxed-string headers carry arbitrary `u8` values, including high bits). `String::from_utf8_unchecked` is the only way to satisfy `StringAttribute::new`'s `&str` parameter without an extra copy. The downstream consumer (MLIR's StringAttribute) treats the data as opaque bytes.

**Verdict**: justified by the API mismatch between Rust's UTF-8-safe `String` and MLIR's byte-attribute primitive.

## Risk assessment

- **Bridge FFI unsafes** (6): risk = correctness depends on HIR's per-builtin arg-kind validation never letting a non-String reach a String-typed bridge fn. The validation is exhaustive at HIR-lower time per ADRs 0110 / 0113.
- **Value lifetime transmutes** (24): risk = correctness depends on melior's API contract holding (`Value` covariant in second lifetime). melior's API hasn't broken this in any version we've tracked.
- **String-attribute carrier** (1): risk = LLVM stops accepting non-UTF-8 in string attributes. LLVM's C++ representation is `StringRef = (const char*, size_t)` which is byte-level; the Rust binding's `&str` annotation is the constraint, not LLVM's underlying behaviour.

All three risk categories are bounded and have clear escalation paths if future evidence demands.

## Decision

No code changes. The audit is the deliverable.

Future ADRs may revisit individual sites if:
- A new melior version eliminates the lifetime invariance constraint (remove 24 transmutes).
- The bridge widens to non-FFI marshaling (revisit the 6 bridge unsafes if a higher-level wrapper replaces raw `*const u8`).
- LLVM's binding pins a UTF-8 invariant on string attributes (replace the `from_utf8_unchecked` with an explicit byte-array primitive).

## Tests

No new tests. The audit is metadata.

## Test count delta

```
Step 0:  1617 (after ADR 0248)
C1 (docs only): 1617 → 1617
```

## References

- [ADR 0193](0193-phase4-entry-criteria.md) — Phase 4 workstream that this ADR closes.
- [ADR 0195](0195-phase5-entry-criteria.md) — Phase 5 entry; security review item references this audit.
- [ADR 0112](0112-string-abi-refactor.md) — boxed-string-object layout consumed by the bridge unsafes.
- [ADR 0184](0184-gc-type-meta-size-guard.md) — 4 GiB GC payload cap that bounds the slice-from-raw-parts argument.
- [ADR 0243](0243-bridge-error-propagation.md) — `lumelir_raise_error` external linkage hook.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M15 milestone.
