# 0086. Phase 2.6b-hash-key-nan: Hash Key NaN Runtime Diagnostic

- **Status:** Accepted (helper portion superseded by ADR 0087)
- **Kind:** Feature Memo
- **Date:** 2026-05-07
- **Deciders:** ShortArrow

> **ADR 0087 supersede note (2026-05-10):** the
> `emit_hash_key_nan_preflight` helper described below has been
> folded into the new `emit_hash_key_runtime_validity_gate` along
> with ADR 0084's nil trap. The 3 raw-f64 NaN preflight call sites
> (`emit.rs:2766` / `:6554` / `:4339`) using
> `emit_table_index_nan_trap_if` are unaffected. User-visible
> behaviour ("table index is NaN") is preserved.

## Context

Lua spec §3.4.5 forbids NaN as a table index. ADR 0079 (hash-key
dispatch) and ADR 0084 (TaggedValue-key IndexAssign / Index) both
documented `LIC-2.6b-hash-key-nan-runtime-1` as pending: a NaN-
tagged Number key fails `cmpf Oeq` deep inside the hash probe, so
the probe always misses and surfaces as the generic
`s_table_missing_key` trap. The static Number-key array path is
worse — `f2i(NaN)` is implementation-defined in MLIR/LLVM, and the
subsequent bounds check could misbehave silently.

ADR 0086 closes the gap with a dedicated runtime trap, mirroring
ADR 0084's `s_table_index_nil` shape. Parser / HIR are unchanged;
the work is entirely in `src/codegen/emit.rs`.

Codex pre-ADR-0086 review picked Option A (NaN diagnostic) over
B (TaggedValue arith coerce) and C/D (closure feasibility) and
strongly recommended:

- **Centralise the TaggedValue check** at the hash-probe entry
  rather than per-call site — `emit_hash_probe_loop` is the
  single chokepoint for IndexAssign / Index / iterator-internal
  callers.
- **Use `cmpf Une self-self`** for NaN detection — qNaN / sNaN /
  ±NaN agnostic, already proven in `emit_tonumber_for_arith`
  (ADR 0077).
- **Cover the static Number-key array path too** — `t[0/0]` is
  the simplest user-reachable case and has its own `f2i` blast
  radius before any hash code is touched.
- **Trap before** any side effect on the table (no allocation, no
  bucket scan, no value store).

## Decision

### New global `s_table_index_nan`

`emit_module_unverified` adds a sibling to ADR 0084's
`s_table_index_nil`:

```rust
emit_string_global(
    context, module, i8_type,
    "s_table_index_nan",
    "table index is NaN\0",
    loc,
);
```

### Helpers

Two small helpers in `src/codegen/emit.rs`:

```rust
fn emit_table_index_nan_trap_if<'a, 'c>(
    context, block, cond_i1, types, loc,
);

fn emit_hash_key_nan_preflight<'a, 'c>(
    context, block, search_key_slot, types, loc,
);
```

`emit_table_index_nan_trap_if` is a literal copy of ADR 0084's
nil-trap shape (`scf.if cond { printf + exit(1) } else { yield }`).

`emit_hash_key_nan_preflight` reads `tag = slot+0`; the outer
`scf.if (tag == TAG_NUMBER)` only descends into the f64 NaN check
on the then branch (loads `payload = slot+8`, runs `cmpf Une`,
delegates to the trap helper). Other tags skip cleanly with
zero overhead beyond a tag load + compare.

### 4 insertion sites

| Site                                     | When                                     |
|------------------------------------------|------------------------------------------|
| `IndexAssign` Number-key arm             | every `t[expr] = v` write where `expr` is statically Number |
| `Index` Number-key arm                   | every `t[expr]` read where `expr` is statically Number and consumed as a value |
| `emit_local_init_tagged` Number-key arm  | inline `print(t[expr])` / `tostring(t[expr])` paths (ADRs 0065 / 0070) |
| `emit_hash_probe_loop` entry             | every TaggedValue-key call (`Index*` / iterator-internal probes) |

The first three insertions are direct: load `key_f`, compute
`is_nan = cmpf Une key_f key_f`, call
`emit_table_index_nan_trap_if`, then continue with the existing
`f2i` and bounds-check.

The fourth insertion sits at the top of `emit_hash_probe_loop`,
which is the single chokepoint for both
`emit_hash_probe_for_insert` and `emit_hash_probe_lookup`. A
preflight here automatically covers every call site that builds
or reuses a tagged search-key slot, including ADR 0084's
TaggedValue-key IndexAssign / Index arms and the iterator
helpers that ADRs 0080 / 0081 wired up.

### Why not a single helper for the array path too?

The array path has a Number-typed `key_f` directly available; it
does not need the tag dispatch the TaggedValue path needs.
Reusing `emit_hash_key_nan_preflight` there would require
materialising a tagged slot first — pure overhead. The 7-line
`cmpf Une + emit_table_index_nan_trap_if` block in each Number
arm is shorter and reads as a peer to the existing bounds-check.

### Design invariants (Codex review §4)

- **`cmpf Une self-self` is true iff the value is NaN.** Agnostic
  to qNaN / sNaN / ±NaN — Lua's spec wording does not distinguish.
- **Trap fires before any subsequent op on the key.** Static
  Number arms: before `f2i` (which is implementation-defined on
  NaN). TaggedValue arms: before any bucket walk / payload re-
  interpretation.
- **`s_table_index_nan` is distinct from `s_table_missing_key`.**
  Users debugging `t[expr] = v` where `expr` silently became NaN
  no longer have to chase a generic missing-key message.
- **`s_table_index_nan` is distinct from `s_table_index_nil`.**
  ADR 0084's nil trap stays exactly where it was; the two
  diagnostics layer cleanly.

## Alternatives Considered

- **Trap inside `emit_hash_key_eq_dispatched`**: would catch the
  TaggedValue path, but at a per-bucket cost (the eq runs in the
  probe loop body). Rejected — preflight at the probe entry pays
  exactly once per call.
- **Promote NaN to a TAG_NIL slot transparently**: would let
  `t[NaN]` silently become `t[nil]` and then trap with the nil
  message. Rejected — Lua's spec explicitly distinguishes NaN
  from nil; merging diagnostics loses information for the
  common debugger workflow.
- **Defer the static-Number-key array path** (do TaggedValue-only
  in this ADR): rejected. `t[0/0]` is the most reachable user
  case, and the marginal cost (one `cmpf` per arm) is tiny next
  to the existing bounds-check.

## Consequences

- **`LIC-2.6b-hash-key-nan-runtime-1` → resolved.**
- **Test totals: 959 → 965 green.** 6 new e2e in
  `tests/phase2_6b_hash_key_nan.rs`: static Number write trap,
  static Number read trap, TaggedValue NaN via ADR 0074 widening
  trap, regression Number key still works, regression TaggedValue
  string key still works, regression nil trap still fires.
- **LIC totals: 25 / 0 / 3 → 26 / 0 / 2** (resolved / partial /
  pending).
- **Source LOC**: codegen `+95` (1 global, 2 helpers, 4 site
  insertions). Tests `+150`. ADR 0086 + SoT updates `+250`.

### Carry-overs

- **String / Bool / Function / Table key NaN**: not a concept —
  these tags carry no f64 payload that can be NaN. The preflight
  is a no-op for them by construction.
- **NaN-tagged TaggedValue with non-Local source**: same scope
  as ADR 0084 — non-Local TaggedValue keys are still
  `CodegenError::UnsupportedExpr`. The preflight at
  `emit_hash_probe_loop` covers any future Local-or-tmp
  materialisation automatically.

## Documentation updates

- [x] §1 slot layout — n/a (no new tag).
- [x] §2 producer / source taxonomy — n/a.
- [x] §3 consumer matrix — n/a.
- [x] §4 LIC consolidation — `hash-key-nan-runtime-1` moved to
      Resolved. Totals: **27 entries — 26 resolved / 0 partial /
      2 pending**.
- [x] §5 runtime tag invariants — n/a.
- [x] §6 cross-reference — new "Hash key NaN trap" subsection
      pointing at the four insertion sites.
- [x] §7 open questions — `hash-key-nan-runtime-1` removed; ADR
      0083 (Full closures, deferred) remains #1; the
      TaggedValue-arith-coerce candidate (LIC-2.7p-arith-coerce-
      tagged-1) is the next candidate.
- [x] §8 ADR index — ADR 0086 row added; "Last updated" stamp
      bumped.

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative
list (ADR 0068).
