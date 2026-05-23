# 0069. Phase 2.6c-tag-defensive-trap: Trap on Unknown Tagged Slot Tag

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-05
- **Deciders:** ShortArrow

## Context

Codex review (post-ADR-0068) flagged that the TaggedValue
runtime dispatch helpers carry several **defensive `else`
fallbacks** that silently mis-identify an unsupported tag as a
supported one:

- `emit_type_tagged_local` — falls through to `s_typename_number`
  for any tag not in `{Nil, Number, Bool, String}` (ADR 0067).
- `emit_tostring_tagged_local` — falls through to
  `s_typename_function` (ADR 0067).
- `emit_tagged_eq_local_local` — falls through to `i1 false`
  (ADR 0066).
- `emit_print_tagged_local` — falls through to `s_nil` for any
  non-`{Number, Bool, String}` tag, which conflates the
  legitimate Nil arm with future Function / Table tags
  (ADR 0064).

Today every fallback is unreachable because HIR rejects
Function / Table values in tables (LIC-2.6c-tag-hetero-fn-tbl-1).
But when that LIC is closed and tables can carry Function /
Table values, the silent fallbacks would turn the new tag
values into wrong-but-plausible answers — bugs that pass tests
and look correct in `print`. Codex framing: *release で trap/
log、debug で assert が妥当。少なくとも「unknown を function
扱い」は早めに潰すべき。*

This sub-phase replaces all four defensive fallbacks with a
**fail-fast trap**, turning the risk surface from "silent
miscompile in a future sub-phase" into "runtime trap with the
existing `s_table_type_mismatch` diagnostic."

User-visible: no change today (paths unreachable). The trap
becomes observable the moment a follow-up sub-phase lowers
Function / Table values without updating the consumer dispatch
chains.

LIC tracker: no new entries; no resolution either. The
`tagged-semantics.md` invariant on defensive fallbacks (§5
invariant 6) is upgraded from a non-binding comment into the
implemented contract.

## Decision

### Single trap helper

```rust
fn emit_tagged_unknown_tag_trap<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    types: &Types<'c>,
    loc: Location<'c>,
) {
    let msg_ptr =
        emit_addressof(context, block, "s_table_type_mismatch", types, loc);
    emit_exit_with_message(context, block, msg_ptr, types, loc);
}
```

Reuses `s_table_type_mismatch` (ADR 0059 / 0060) so the runtime
diagnostic is consistent with the existing array / hash
type-mismatch trap surface — no new global string, no new error
class.

### Four call sites

For each helper's `str_else` (or equivalent), the previous
silent yield is preceded by a `emit_tagged_unknown_tag_trap`
call. A **placeholder yield** of the original type stays in
place to satisfy `scf.if`'s yield-type contract; the placeholder
is unreachable because `exit(1)` runs first.

| Helper                          | Before                                | After                                  |
|---------------------------------|---------------------------------------|----------------------------------------|
| `emit_type_tagged_local`        | yield `s_typename_number`             | trap; yield `s_typename_number` (placeholder) |
| `emit_tostring_tagged_local`    | yield `s_typename_function`           | trap; yield `s_typename_function` (placeholder) |
| `emit_tagged_eq_local_local`    | yield `i1 0`                          | trap; yield `i1 0` (placeholder)        |
| `emit_print_tagged_local`       | print `s_nil` (also for unknown tags) | inner `is_nil` check: print `s_nil` if Nil, else trap |

`emit_print_tagged_local` needed slightly more care: its
`string_else` previously also handled the legitimate Nil arm
(structurally Nil sat outside the chain). The new code adds a
`is_nil` scf.if inside `string_else` to keep Nil's "print 'nil'"
behaviour while routing any other unknown tag into the trap.

### What stays unchanged

- HIR-guarded `unreachable!()` in compile-time match arms
  (e.g. `_ => unreachable!("HIR rejects non-Number/...")`) —
  those guard producer-side type invariants, not runtime tag
  dispatch, and remain Rust-level invariants.
- `emit_value_slot_check_number` — already traps with the same
  diagnostic on tag mismatch in the locals-extract path
  (ADR 0059); its semantics are unchanged.
- The `ValueKind::TaggedValue` static-dispatch fallback in
  `Builtin::Type` (when the operand is not a `Local`) keeps
  returning `s_typename_number` — that path is for non-Local
  TaggedValue producers, and currently no such producer exists.
  When function-return widening lands, that arm needs its own
  treatment.

### CA invariants preserved

| Layer    | Change                                                   |
|----------|----------------------------------------------------------|
| Lexer    | None                                                     |
| Parser   | None                                                     |
| AST      | None                                                     |
| HIR      | None                                                     |
| Codegen  | One new helper, four `str_else` rewrites in `emit.rs`.   |
| Tests    | New `tests/phase2_6c_tag_defensive_trap.rs` (3 tests)    |
| Docs     | `tagged-semantics.md` §3 + §5 updates                    |

## Alternatives Considered

- **Add a dedicated `s_tagged_unknown_tag` global string.**
  Sharper diagnostic, but introduces a new error class for a
  surface that's currently unreachable. Reuse keeps the LLVM
  string pool small and the diagnostic surface uniform.
  Reject — premature for an unreachable path.
- **Promote each fallback to `unreachable!()` at codegen time.**
  Would crash the *compiler* on a future bug instead of the
  *generated program*. The current trap is the right level: a
  user invoking the future-bug code gets a diagnostic, not a
  compiler ICE. Reject.
- **Conditionally compile traps under a `debug` cfg.** Codex
  mentioned "release で trap/log、debug で assert" — but the
  generated MLIR/LLVM has no `cfg(debug_assertions)` analogue.
  An always-on trap with a cheap implementation is the
  pragmatic substitute.
- **Fix the broader `emit.rs` cohesion now (tagged.rs split).**
  Codex priority #1, deferred from ADR 0067, still deferred
  here. The trap fix is small enough to land cleanly without
  the split, and the split benefits from the trap unification
  being settled first.

## Consequences

- ~80 LOC across `src/codegen/emit.rs` (new helper + 4 site
  rewrites).
- 3 new e2e tests in
  `tests/phase2_6c_tag_defensive_trap.rs` that assert the HIR-
  level Function / Table value rejection is still in place.
  These are negative tests by design — they pin the assumption
  that the trap remains unreachable until a deliberate
  sub-phase removes the HIR rejection.
- 819 + 3 = **822** tests green. Zero regressions in existing
  matrix / consumer / eq / nil suites.
- `tagged-semantics.md` §3 and §5 updated to describe the
  uniform trap behaviour. The "fallback observability" item
  comes off Codex's open-questions list (operationally
  resolved; doc-side observability is an ADR 0068 follow-up).
- Future Function / Table tag work (LIC-2.6c-tag-hetero-fn-
  tbl-1) inherits a clear contract: dispatch on the new tag
  in every consumer, or the runtime trap fires.

## Lua-Incompatibility Tracker

No change. See `docs/design/tagged-semantics.md` §4 for the
authoritative consolidated list (ADR 0068 / Codex review #2).

## Out of Scope

- **`tagged.rs` module split** — Codex priority #1, still
  deferred. Trap fix lands without it.
- **Function / Table values in tables** —
  LIC-2.6c-tag-hetero-fn-tbl-1. The trap added here turns the
  silent-miscompile risk into runtime fail-fast for that
  follow-up.
- **`emit.rs` tagged dispatch helper extraction** (Codex's
  "tag → runtime helper selection" pure-helper pattern). Out
  of scope; revisit during the tagged.rs split.
- **ADR template "tagged semantics 更新要否" checkbox** —
  Codex's CI / PR-checklist suggestion. Operational; track
  separately.
