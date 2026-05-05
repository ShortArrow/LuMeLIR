# 0067. Phase 2.6c-tag-consumers: TaggedValue Runtime Dispatch for `type` and `tostring`

- **Status:** Accepted
- **Date:** 2026-05-05
- **Deciders:** ShortArrow

## Context

ADR 0066 closed the last open hole in TaggedValue equality.
Codex review (post-0066) recast priorities: with semantics
closing, the remaining work is making **read consumers** match
the runtime tag instead of the static `ValueKind`. The
already-known LIC entry was:

- `LIC-2.6c-tag-locals-1` (ADR 0063): `type(x)` for a
  `Local(TaggedValue)` returns the static `"number"` regardless
  of the runtime tag. A widening `local x = t[k]` followed by
  `print(type(x))` says `"number"` even when the underlying
  value is a string or boolean.

Codex also flagged `tostring(Local(TaggedValue))` as a closely
related silent-trap surface: `emit_expr` on the operand traps
on non-Number tag, so `tostring(x)` aborts for Bool / String /
Nil payloads. Strictly that's `LIC-2.6c-tag-locals-1`'s
sibling, but ADR 0063 didn't separately log it.

This sub-phase resolves both surfaces with a uniform
"detect `Local(TaggedValue)` operand → call a tag-dispatching
helper" pattern. Arithmetic / comparison consumers stay
trapping by design: Lua spec says `nil + 1` and `"a" < 1` are
runtime errors, so the existing trap is correct.

User-visible:
```lua
local t = {1, "hello", true}
local a = t[1]
local b = t[2]
local c = t[3]
print(type(a), type(b), type(c))           -- number  string  boolean
print(tostring(a), tostring(b), tostring(c)) -- 1  hello  true
```

## Decision

### `Builtin::Type` arm — Local(TaggedValue) special case

Before falling through to the static-kind dispatch, the `type`
builtin checks for the `(Local, TaggedValue)` shape and calls a
new `emit_type_tagged_local(slot_ptr)` helper:

```rust
if kind == ValueKind::TaggedValue {
    if let HirExprKind::Local(LocalId(idx)) = &args[0].kind {
        return Ok(emit_type_tagged_local(slot_ptr, …));
    }
}
// existing static dispatch path (Function/Table/Number/etc.)
```

`emit_type_tagged_local` is a 4-way `scf.if` chain over the
slot's tag at offset 0:

| tag value      | yields                          |
|----------------|---------------------------------|
| TAG_NIL (0)    | `s_typename_nil`                |
| TAG_NUMBER (1) | `s_typename_number`             |
| TAG_BOOL (2)   | `s_typename_boolean`            |
| TAG_STRING (3) | `s_typename_string`             |
| else           | `s_typename_number` (defensive) |

The existing static `ValueKind::TaggedValue → s_typename_number`
arm (the one ADR 0063 left as a deliberate shortcut) becomes
unreachable in practice — it stays as a defensive fallback for
any future TaggedValue source that is not a Local.

### `Builtin::ToString` arm — Local(TaggedValue) special case

Same pattern: detect the `(Local, TaggedValue)` shape, call
`emit_tostring_tagged_local(slot_ptr)`. The helper dispatches
on tag and yields a `ptr` to the string representation:

| tag value      | strategy                                                |
|----------------|---------------------------------------------------------|
| TAG_NIL        | `s_nil` global                                          |
| TAG_NUMBER     | `emit_tostring(payload f64, Number)` — snprintf `%g`    |
| TAG_BOOL       | `select(payload != 0, s_true, s_false)`                 |
| TAG_STRING     | `payload ptr` directly                                  |
| else           | `s_typename_function` (defensive)                       |

The Number branch reuses the existing `emit_tostring(... Number)`
codegen so format details (precision, NaN, etc.) match plain
`tostring(<number-typed>)` exactly.

### Other consumers

- **`print`** — already runtime-dispatches `Local(TaggedValue)`
  via `emit_print_tagged_local` (ADR 0064) and `Index`
  through a tmp tagged slot (ADR 0065). No change.
- **`==` / `~=`** — fully resolved by ADR 0065 (Local-Literal)
  and ADR 0066 (Local-Local). No change.
- **`isnil` (`x == nil`)** — `IsNil(Box<HirExpr>)` (ADR 0066).
  No change.
- **`..` concat** — Phase 2.7c (ADR 0026) auto-coerces non-
  String operands through `tostring(x)`. With the new
  `tostring` runtime dispatch, concat now correctly stringifies
  TaggedValue locals.
- **Arith / comparison** — stay trapping. Lua spec: `nil + 1`,
  `"a" < 1`, etc. are runtime errors. The existing extract-and-
  trap-on-non-Number path matches.

### CA invariants preserved

| Layer    | Change                                                                          |
|----------|---------------------------------------------------------------------------------|
| Lexer    | None                                                                            |
| Parser   | None                                                                            |
| AST      | None                                                                            |
| HIR      | None — purely codegen-side dispatch                                             |
| Codegen  | Two new helpers (`emit_type_tagged_local`, `emit_tostring_tagged_local`); two `Builtin` arms gain a Local(TaggedValue) special case ahead of the static path. |

## TDD Process

1. **Step 1 — Red.** 15 e2e tests in
   `tests/phase2_6c_tag_consumers_matrix.rs` over four
   consumers (`type`, `tostring`, `..` concat, regression for
   existing surfaces) × four runtime tags (Number / Bool /
   String / Nil). 7 fail outright (the new behaviour), 8
   already pass (regression coverage).
2. **Step 2 — Green.** Two new helpers in `emit.rs`; two
   Builtin arms add the special case. All 15 pass; total green
   at 819 (= 804 + 15).
3. **Step 3 — ADR + AGENTS + commit.**

The matrix file (`phase2_6c_tag_consumers_matrix.rs`) is
explicitly framed as a *scaffold*: subsequent phases adding new
TaggedValue consumers (or the deferred Function/Table tags)
should grow the cells, not start a new file. This addresses
Codex's "consumer matrix test 不在" technical-debt note.

### Tidy First (`tagged.rs` module split) — deferred

The plan originally proposed factoring tagged-value codegen
into a separate `src/codegen/tagged.rs` module before this
feature. On inspection the cross-module dependency surface
(string globals, libc helpers, hash probe helpers, type
constants) is large enough that a clean split warrants a
dedicated phase. The mini-helper extraction (`emit_tag_load`
etc.) is left as a small follow-up Tidy First, captured in the
roadmap below.

## Alternatives Considered

- **Make `infer_kind(HirExprKind::Local with TaggedValue)`
  context-sensitive**, returning the runtime kind via a side
  channel. Defeats the entire static type system and pushes
  dispatch up into HIR; rejected.
- **Generate a closure-based 4-way dispatch helper** to avoid
  duplicating the `scf.if` chain across `print` / `eq-local-
  local` / `type` / `tostring`. Melior's lifetime-bound
  Region/Block APIs make closure passing awkward; the
  duplication is manageable for now. Rejected for this round;
  reconsider when a fifth `TaggedValue` consumer appears.
- **Auto-coerce `arith` / `cmp` to runtime dispatch on tagged
  operands** ("`x + 1` where `x` is String tag converts via
  `tonumber(x)`"). Lua does this for some arithmetic; mostly
  orthogonal and still spec-correct to trap right now.
  Rejected for this phase.

## Consequences

- Codegen: ~280 LOC across two helpers in `emit.rs`. No HIR
  changes, no AST/parser/lexer churn.
- 15 new e2e tests; total green at **819** (= 804 + 15). Zero
  regressions across the existing suite.
- **`LIC-2.6c-tag-locals-1` → resolved.** `type(x)` now
  reflects the runtime tag for any widened local.
- A second silent-trap surface (`tostring(x)` for non-Number
  tagged locals) is closed in the same phase. No new LIC
  entry — it shares scope with the original.
- The matrix-test file is the new safety net for future
  consumer additions. Its cells are deliberately low-cost so
  later phases (`type`/`tostring` for Function/Table tags,
  iteration, etc.) can extend the grid without rewriting.

## Lua-Incompatibility Tracker (cumulative)

Updated from ADR 0066.

| ID | Behaviour | Lua spec | Our behaviour | Status |
|----|-----------|----------|---------------|--------|
| LIC-2.6a-arr-1 | OOB read | returns nil | inline `print(t[oob])` prints nil; non-print uses still extract f64 | resolved (ADR 0061 + 0063 + 0065) |
| LIC-2.6a-arr-2 | element kinds | heterogeneous | Number/Bool/String/Nil supported | resolved Bool/String (ADR 0064); Function/Table → LIC-2.6c-tag-hetero-fn-tbl-1 |
| LIC-2.6a-arr-3 | key kinds | any | Number+String | partial (ADR 0058) |
| LIC-2.6a-wr-1 | hole write | creates a hole | creates a hole (Nil-tagged) | resolved (ADR 0059) |
| LIC-2.6a-wr-2 | grow write | extends length | extends length | resolved (ADR 0057) |
| LIC-2.6a-wr-3 | array value kinds | heterogeneous | Number/Bool/String supported | resolved Bool/String (ADR 0064); Function/Table → LIC-2.6c-tag-hetero-fn-tbl-1 |
| LIC-2.6b-hash-1 | missing key read | returns nil | inline print: nil; non-print uses still trap on extract | resolved (ADR 0061 + 0063 + 0065) |
| LIC-2.6b-hash-2 | hash value kinds | heterogeneous | Number/Bool/String/Nil-delete supported | resolved Bool/String (ADR 0064); Function/Table → LIC-2.6c-tag-hetero-fn-tbl-1 |
| LIC-2.6c-tag-hash-1 | `t.k = nil` | physically removes the key | sentinel + rehash drops | resolved (ADR 0062) |
| LIC-2.6c-tag-locals-1 | `type(x)` for widened local | runtime dispatch on actual tag | runtime tag dispatch (`type` + `tostring`) | **resolved (this ADR)** |
| LIC-2.6c-tag-hetero-fn-tbl-1 | Function/Table table values | accepted | rejected at HIR | new (ADR 0064) |
| LIC-2.6c-tag-hetero-eq-1 | `==`/`~=` between two TaggedValue locals | runtime tag-aware | runtime tag dispatch | resolved (ADR 0066) |
| LIC-2.6c-tag-hetero-inline-1 | inline `print(t[k])` for hetero values | prints typed value or "nil" | runtime tag dispatch | resolved (ADR 0065) |

## Out of Scope

- **`tagged.rs` module split** — deferred to a dedicated
  Tidy First phase (Codex roadmap).
- **`docs/design/tagged-semantics.md` SoT** — same.
- **Function/Table values in tables** —
  `LIC-2.6c-tag-hetero-fn-tbl-1`.
- **String→Number coercion on arith** (`"5" + 1`) — Lua-spec
  feature, mostly orthogonal.
- **Function-return TaggedValue widening** —
  `local x = f()` where `f` returns nil/heterogeneous.
- **Inline `type(t[k])` and `tostring(t[k])`** — current
  scope is `Local(TaggedValue)` only. Inline forms still go
  through the static `Index` path.
- **Iteration `pairs` / `ipairs`**.
