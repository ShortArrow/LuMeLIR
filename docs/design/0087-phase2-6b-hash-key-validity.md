# 0087. Phase 2.6b-hash-key-validity: Probe Chokepoint Owns Hash-Key Runtime Validity Policy

- **Status:** Accepted
- **Date:** 2026-05-10
- **Deciders:** ShortArrow

## Context

After ADR 0086, every TaggedValue-key hash probe ran two
independent runtime-validity checks:

| Check | Owner | Location |
|---|---|---|
| TAG_NIL → trap `s_table_index_nil` | inline at call site | `emit.rs:3160-3195` (IndexAssign), `emit.rs:6723-6757` (Index) |
| f64 payload NaN → trap `s_table_index_nan` | chokepoint | `emit_hash_probe_loop` entry (`emit.rs:5535`) |

The split was historical: ADR 0084 added the nil trap inline at
the source-level arms because no chokepoint existed yet, and ADR
0086 introduced the chokepoint exclusively for NaN. The
consequence was that every new TaggedValue-key carrier (e.g. a
future generic-for that builds a tagged search-key slot from a
non-`Local` source) had to remember to add an inline nil trap, or
silently regress to the generic `s_table_missing_key` message.

`codex review` flagged the duplication as the highest-leverage
follow-up to ADR 0086 (5/6 perspectives picked it; the sixth —
non-ad-hoc — escalated the framing to "hash-key runtime validity
policy" rather than "nil + NaN preflight unification" so future
tag kinds (WeakKey / ThreadKey) can extend the matrix without
revisiting the chokepoint shape).

## Decision

### Pure decision: `tagged.rs::policy_for_tag`

A new module-level enum and pure function in `src/codegen/tagged.rs`:

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum HashKeyValidityPolicy {
    TrapNil,
    CheckNaN,
}

pub(crate) fn policy_for_tag(tag: i64) -> &'static [HashKeyValidityPolicy] {
    match tag {
        TAG_NIL => &[HashKeyValidityPolicy::TrapNil],
        TAG_NUMBER => &[HashKeyValidityPolicy::CheckNaN],
        _ => &[],
    }
}
```

The function is total, deterministic, and depends on nothing
beyond the tag constants — testable in isolation without an MLIR
`Context`. Future tag kinds extend the matrix here; call sites
need no change.

### Effectful executor: `emit.rs::emit_hash_key_runtime_validity_gate`

Replaces ADR 0086's `emit_hash_key_nan_preflight`. Walks the
fixed `POLICED_TAGS = [TAG_NIL, TAG_NUMBER]` list (order is
load-bearing — TAG_NIL has no f64 payload, so the NaN load must
not run on it), consults `policy_for_tag` for each, and emits a
guarded `scf.if` whose then-branch lays down the policy-specific
trap. The trap message globals (`s_table_index_nil`,
`s_table_index_nan`) and the underlying
`emit_table_index_nan_trap_if` shape stay owned by `emit.rs` —
`tagged.rs` knows only the policy decision.

### Chokepoint migrate

`emit.rs:5535` swaps `emit_hash_key_nan_preflight` for
`emit_hash_key_runtime_validity_gate`. Every probe wrapper
(`emit_hash_probe_lookup`, `emit_hash_probe_for_insert`)
inherits the gate transparently.

### Inline trap retire

The two ADR 0084 inline nil traps are removed:

- `emit.rs:3160-3195` — IndexAssign TaggedValue arm (33 lines)
- `emit.rs:6723-6757` — Index TaggedValue arm (35 lines)

Both arms continue through `emit_hash_probe_for_insert` /
`emit_hash_probe_lookup` after the removal, so the chokepoint
fires before any bucket walk. ADR 0084's user-visible behaviour
("table index is nil" on TaggedValue-key nil) is preserved.

### Ownership boundary

Doc-comments on `emit_hash_probe_for_insert` /
`emit_hash_probe_lookup` and on the `s_table_index_nil` /
`s_table_index_nan` globals state explicitly: validity policy
decisions belong in `tagged.rs::policy_for_tag`; trap message
globals and the effectful emitter belong in `emit.rs`; callers
must not add inline nil/NaN checks before the chokepoint. The
note prevents future drift — a reader extending the codegen
sees the chokepoint's authority before reaching for an inline
duplicate.

## Alternatives Considered

- **Single `match` inside the gate (no `policy_for_tag`).** The
  match would conflate decision and emission inside one MLIR-
  bound function. Rejected — Codex review v2 (FP perspective)
  flagged this as the critical issue: a pure decision module
  unlocks unit testing without MLIR `Context` and lets future
  tag matrices change without touching codegen.
- **Repurpose `emit_hash_key_nan_preflight` to also handle
  TAG_NIL.** Smallest diff, but keeps "preflight" naming that
  reads as NaN-only. Rejected — non-ad-hoc framing wants the
  helper named for its policy ("runtime validity") not for one
  current member.
- **Migrate the 3 raw-f64 NaN preflight sites
  (`emit.rs:2766` / `:6554` / `:4339`)** to the new gate. They
  do not handle a tagged slot — `key_f` is already an f64 — so
  the gate would have to either accept a generalised input
  shape or wrap them in a fake tagged slot. Both approaches
  trade clarity for false uniformity. Rejected — the 3 raw-f64
  sites stay on the existing `emit_table_index_nan_trap_if`
  helper.
- **Fold the Index TaggedValue arm's missing-key bug fix into
  this ADR.** The arm uses `emit_hash_probe_lookup`
  (`trap_on_null=true`) which traps on missing keys, violating
  Lua §3.4.5 ("missing key returns nil"). Tracked separately as
  `LIC-2.6b-hash-missing-key-read-1`; out of scope here. The
  fix likely needs a `for_insert + key_at_null => nil` mirror
  in the read arm, which is structurally distinct from the
  validity-policy unification this ADR addresses.

## Consequences

- **`LIC-2.6b-hash-key-nil-runtime-1` → resolved.** ADR 0079 +
  ADR 0084 + ADR 0087 form the full lineage.
- **Test totals: 990 → 995 green.** 3 new pure unit tests in
  `tagged.rs` cover the decision matrix; 2 new e2e tests in
  `tests/phase2_6b_hash_key_nil.rs` pin both probe entries
  (insert / lookup) for TAG_NIL.
- **LIC totals: 26 / 1 / 2 → 27 / 0 / 2** (resolved / partial /
  pending). New `LIC-2.6b-hash-missing-key-read-1` tracks the
  Index TaggedValue arm's missing-key bug discovered during this
  ADR's review.
- **Source LOC.** Codegen: `+~30` (decision module + executor)
  net of `-68` (inline traps removed) = **net `-38`**. Tests:
  `+~80`. ADR 0087 + SoT updates: `+~150`.
- **ADR 0086 partial supersede.** ADR 0086's
  `emit_hash_key_nan_preflight` helper is folded into the new
  gate. The 3 raw-f64 NaN preflight sites
  (`emit.rs:2766` / `:6554` / `:4339`) using
  `emit_table_index_nan_trap_if` are unaffected.

### Carry-overs

- **`LIC-2.6b-hash-missing-key-read-1` (new).** The Index
  TaggedValue arm at `emit.rs:6700-6800` uses
  `emit_hash_probe_lookup` (trap_on_null=true), so a missing
  key fires `s_table_missing_key` instead of returning nil per
  Lua §3.4.5. Symmetrical to the IndexAssign arm which uses
  `emit_hash_probe_for_insert` (trap_on_null=false). Fix is a
  small structural mirror but separable from validity policy.
- **Future tag kinds (WeakKey, ThreadKey, …).** Extend
  `policy_for_tag` and `POLICED_TAGS` only; the gate shape and
  call-site contract stay frozen.

## TDD Process

1. **Red.** 3 pure unit tests in `tagged.rs::tests` for the
   decision matrix (`policy_for_tag(TAG_NIL)` /
   `policy_for_tag(TAG_NUMBER)` / pass-through tags). Could not
   reference unimplemented symbols at HEAD; written together
   with Step 1 implementation in a single file edit. 2 e2e
   regression-pin tests in `tests/phase2_6b_hash_key_nil.rs`
   cover both probe entries — green Day 0 (inline traps catch
   them) and must remain green Day 4+ (chokepoint catches them
   after migration).
2. **Green.** Step 1 (decision module) → 990 + 3 = 993. Step 2
   (executor) builds clean. Step 3 (chokepoint migrate) keeps
   993 green — the new gate runs alongside the inline traps;
   nothing user-visible changes. Step 4 (inline trap retire)
   keeps 993 green — the chokepoint now catches what the inline
   traps used to. Step 0b e2e add → **995 green**.
3. **Refactor.** None needed — the per-policy emission stays
   inline in the gate's `match` arm. Extracting an
   `emit_validity_policy_body` helper would trade two lines for
   a four-arg call; rule-of-three not met (2 policies today).

## Documentation updates

- [x] `docs/design/tagged-semantics.md` §6 (new section: "Hash-
      key runtime validity policy") — policy table, chokepoint
      location, ownership boundary.
- [x] `docs/design/tagged-semantics.md` §3 — helper inventory:
      `emit_hash_key_runtime_validity_gate` / `policy_for_tag`.
- [x] `docs/design/tagged-semantics.md` §4 — `LIC-2.6b-hash-
      key-nil-runtime-1` moved to Resolved. New `LIC-2.6b-
      hash-missing-key-read-1` added to Pending.
- [x] `docs/design/0086-phase2-6b-hash-key-nan.md` — partial
      supersede note appended to Status.
- [x] `AGENTS.md` — Phase 2.6+ row added.

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative
list (ADR 0068).
