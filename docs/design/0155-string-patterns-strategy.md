# 0155. `string` Patterns (`find` / `match` / `gmatch` / `gsub`) — Phase 3 Trigger

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-01
- **Deciders:** ShortArrow

## Context

Last row in [ADR 0133](0133-phase2-completion-criteria.md)'s deferral table. Lua spec §6.4 documents `string.find`, `string.match`, `string.gmatch`, `string.gsub` — a pattern-matching surface that's almost-but-not-quite regex (no alternation, no backrefs by default, simpler character class syntax).

The pattern engine is the largest single design surface remaining in Phase 2. Per ADR 0133's contract ("each workstream has an ADR; implementation may stagger"), this ADR pins implementation to Phase 3.

## Decision

**Phase 2 leaves the pattern surface unimplemented. Phase 3 ports a Lua-spec-compliant pattern matcher.**

Concretely:

1. **Phase 2 = `string.find` / `string.match` / `string.gmatch` / `string.gsub` continue to reject at HIR (`UnknownFunction`).**
2. **Phase 3 = port the Lua 5.4 reference pattern matcher.**
   - Strategy: port the C reference implementation (~600 LOC) into Rust runtime helper, exposed via libc-style externs.
   - HIR: 4 new `Builtin` variants (`StringFind`, `StringMatch`, `StringGmatch`, `StringGsub`).
   - Codegen: dispatch to runtime helpers; each helper is a Rust fn compiled into the binary via `#[no_mangle] extern "C"`.
   - Returns: `StringFind` returns `(Number, Number, captures...)`; `StringMatch` returns `captures...` (or single String); `StringGmatch` returns an iterator function; `StringGsub` returns `(String, Number)`.
   - Multi-return ABI reuses ADR 0021 / 0076.
   - Capture semantics: each `(...)` group in the pattern returns a String capture.
   - Anchors `^` `$`, char classes `%a` `%d` `%s` `%w` `%p` `%l` `%u`, sets `[abc]`, ranges `[a-z]`, complement `[^...]`, quantifiers `*` `+` `-` `?`, balanced matches `%b()`, escaped magic chars `%.` `%%` etc.

3. **Iterator (`gmatch`) integration**: ADR 0085 generic-for + ADR 0080 `pairs` provides the iter protocol. `gmatch` returns an iterator function consumed by `for cap in string.gmatch(s, pat) do ... end`.

### What Phase 3 ADR owns

- The pattern matcher Rust port (~600-1000 LOC) compiled into a runtime library.
- 4 new `Builtin` HIR variants.
- 4 new codegen emit arms.
- Multi-return ABI integration for `find` and `gsub`.
- Iterator ABI integration for `gmatch`.
- Compile-time validation of pattern strings where feasible (constant patterns can have their `(...)` capture count statically computed for arity matching).

### Trigger to implement

Any of:

- (a) User code with regex-like text processing requirements.
- (b) `string.format` (ADR 0152) widens to full Lua spec — `find` / `match` are siblings.
- (c) `_ENV` / true globals (ADR 0154) lands and exposes the `string.*` namespace as a table — patterns belong there.

## Alternatives considered

- **Implement now in Phase 2.** Rejected — the pattern engine is XL sized (largest single Phase 2 implementation work); blocks Phase 2 close for many weeks.
- **Use a Rust regex library (e.g. `regex` crate).** Rejected — Lua patterns are syntactically distinct from PCRE/RE2; users expect Lua semantics. A port of the reference matcher is the right path.
- **Per-fn ADRs (`find` / `match` / `gmatch` / `gsub` each).** Rejected — they share the same matcher core; bundle into a single Phase 3 ADR.
- **Bundle into ADR 0152 `string.format`.** Rejected — format is printf-style (statically validated), patterns are runtime parsed; different concerns.

## Consequences

**Positive**
- ADR 0133's `string` patterns row is satisfied; Phase 2 close criterion advances.
- Phase 3 implementation has a clear scope ceiling.

**Negative**
- User-visible feature gap stays in Phase 2 (no text-processing primitives).
- Phase 3 implementation is XL.

**Locked in until superseded**
- Lua 5.4 pattern semantics (no PCRE / RE2 syntax substitution).
- Port-from-C strategy (no FFI to libpcre or similar).
- Runtime-parsed patterns (no compile-time pattern compilation in Phase 3 first cut).

## Documentation updates

- [x] §4 LIC — new `LIC-string-patterns-strategy-1`.
- [x] §7 — closes `string` patterns open item with Phase 3 trigger.
- [x] §8 — adds 0155.
- [x] ADR 0133 deferral table — patterns row gets `RESOLVED by ADR 0155 (Phase 3 trigger)` annotation.

## Test count delta

```
Step 0:   1366 (after ADR 0152)
C1 (this ADR + SoT updates): 1366 → 1366 (docs only)
```

Decision-only ADR.

## Critical files

- `docs/design/0155-string-patterns-strategy.md` (this file).
- `docs/design/tagged-semantics.md` — §4 / §7 / §8.
- `docs/design/0133-phase2-completion-criteria.md` — deferral row annotation.
- `docs/design/README.md` — index entry.

## Risks

| Risk | Mitigation |
|---|---|
| Port introduces bugs vs reference impl | Phase 3 ADR will adopt the reference C impl's test suite + property-tests. |
| Multi-return ABI for `find` doesn't fit existing single-fn callers | The Phase 3 ADR audits the multi-return ABI before landing; ADR 0021 / 0076 patterns already extend. |
| Pattern matcher hot-loop performance | Out of scope for Phase 3 first cut; if a real workload demands it, a follow-up ADR can JIT-compile patterns. |

## Future work

- Phase 3 implementation ADR (the pattern matcher port).
- Pattern compilation cache (avoid re-parsing per call).
- `%g` (printable except space) and `%G` complement — Lua 5.4 additions.
- UTF-8 patterns (`utf8.*` library) — separate ADR.

## References

- [ADR 0085](0085-phase2-8e-iter-generic.md) — generic-for that `gmatch` plugs into.
- [ADR 0021](0021-phase2-5d-multi-return.md) — multi-return ABI (used by `find` / `gsub`).
- [ADR 0103](0103-phase2-stdlib-string-begin.md) — `string` namespace dispatch.
- [ADR 0133](0133-phase2-completion-criteria.md) — deferral table row this ADR closes.
- [ADR 0152](0152-string-format.md) — sibling `string.format`.
- Lua 5.4 reference manual §6.4.1 — pattern semantics.
