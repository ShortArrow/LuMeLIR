# 0228. `string.find` Literal Substring Search (Plain Mode)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

First M7 sub-ADR. [ADR 0155](0155-string-patterns-strategy.md) decided the long-term path for `string.find` / `match` / `gmatch` / `gsub`: port the Lua reference pattern matcher (~600 LOC) into a Rust runtime helper. That's a multi-session port.

This ADR ships a narrower first cut: `string.find(s, sub)` in **literal substring mode** — `sub` is treated as a plain byte sequence with no magic-char interpretation. Returns 1-indexed start position (TaggedValue Number) on a match or Nil on no-match. Single-return only; multi-return (start, end) and pattern semantics ship in subsequent M7 sub-ADRs.

The cut delivers the most common `string.find` use case (`if string.find(s, fixed_substring) then ...`) immediately while the matcher port lands incrementally.

## Scope (literal)

- ✅ New `Builtin::StringFind` HIR variant. `string_from_method("find") → Some(StringFind)`. Name `"string.find"`, arity `(2, 2)`, `ret_kinds = [TaggedValue]`, `param_kinds_for_arity = [String, String]`. `infer_kind → TaggedValue`.
- ✅ Bridge object adds `extern "C" fn lumelir_string_find_plain(s_ptr, sub_ptr) -> i64`. Reads both ADR 0112 length headers, returns 1-indexed start position or 0.
- ✅ Codegen extern declaration + emit arm. The emit arm allocates a TaggedValue tmp slot; the helper's i64 result is sitofp'd to f64 + stored with TAG_NUMBER on match, or Nil-stored on miss; the slot ptr is returned.
- ✅ `if local_taggedvalue then` non-trapping path: `emit_if` reads the tag at slot offset 0 directly and compares to `TAG_NIL`, skipping the standard `emit_value_slot_check_number` trap. Required for the canonical `local p = string.find(...); if p then ... end` idiom.
- ✅ Empty pattern returns 1 (Lua semantics for empty match at start).
- ❌ Pattern magic chars (`%a` / `%d` / `^` / `$` / `*` / `+` / `?` / `[...]` / `(...)` / `.`). The current implementation does literal byte search; magic-char patterns will silently not match. Pattern matcher port is the second M7 sub-ADR.
- ❌ Multi-return `(start, end)`. Single-return truncation per ADR 0081 Next precedent. M7-B adds the `end` position via multi-assign integration.
- ❌ `init` start-position arg. Lua spec `string.find(s, pat, init)` — future sub-ADR.
- ❌ `plain` arg. Lua spec `string.find(s, pat, init, plain)` lets the user explicitly request the mode this ADR delivers; the arg is deferred until the pattern path also lands so `plain=true` has a non-default mode to disable.
- ❌ Captures `(...)` returning extra values. Captures arrive with the pattern matcher port.
- ❌ TaggedValue truthiness with `TAG_BOOL` payload check. Only `TAG_NIL` is the falsy tag in this implementation; storing a `false` boolean in a TaggedValue slot and then using it as a cond reports truthy. Real Lua semantics check both nil and false. Future widening.
- ❌ Non-`if` cond positions for TaggedValue Locals (`while`, `repeat ... until`, `and` / `or`). Mechanical extension; deferred until first user demand.

## Decision

### Marshaling shape

```mlir
%s   = emit_expr(args[0])     // !llvm.ptr → boxed-string ptr
%sub = emit_expr(args[1])     // !llvm.ptr → boxed-string ptr
%pos = llvm.call @lumelir_string_find_plain(%s, %sub) : i64
%is_found = arith.cmpi ne, %pos, 0_i64
%slot = llvm.alloca i32 2 x i64
scf.if %is_found {
  %f64_pos = arith.sitofp %pos : i64 → f64
  emit_value_slot_store_number(%slot, %f64_pos)
} else {
  emit_value_slot_store_nil(%slot)
}
yield %slot
```

### Non-trapping `if` cond for Local-of-TaggedValue

`emit_if` adds an early branch: when `cond` is `HirExprKind::Local(LocalId)` with `LocalInfo::kind == ValueKind::TaggedValue`, read the tag at the slot's offset 0 (`llvm.load slot : i64`) and compute `tag != TAG_NIL`. This bypasses the standard `emit_value_slot_check_number` trap that fires for any non-Number tag, restoring the spec-compliant truthiness rule for TaggedValue values that hold either a Number or Nil.

The narrower scope: only the Local-Direct-Local case is patched. Other consumers of TaggedValue values (`tostring(p)`, `print(p)`, arithmetic) still go through the existing dispatch paths, which already handle the Nil tag explicitly for those sites (per ADR 0064 / 0067 etc.).

### Why split off the pattern matcher port

The Rust port of `lstrlib.c`'s `match` / `singlematch` / `matchclass` / `matchbalance` etc. is XL work. Shipping the plain-substring path first:

1. Validates the HIR + codegen + bridge plumbing for a TaggedValue-returning builtin that takes two String args.
2. Lifts the TaggedValue-Local-as-cond limitation that future TaggedValue-returning builtins (`pcall` already, `string.match`, etc.) will all need.
3. Delivers ~80% of real-world `string.find` use (fixed-string lookup) at the cost of 10% of the matcher port budget.

## Tests

`tests/phase4_string_find_plain.rs` (NEW, 6 e2e):

1. `string.find("hello world", "world")` → `7`.
2. `string.find("hello", "xyz")` → `nil`.
3. `string.find("abc", "")` → `1` (empty match at start).
4. `string.find("hello", "hel")` → `1` (pattern at start).
5. `if string.find(s, sub) then ... end` matched path → `"found"`.
6. Same `if` shape over the no-match case → `"missed"` (proves the non-trapping TaggedValue cond works).

## Test count delta

```
Step 0:  1519 (after ADR 0227)
C3 (impl + 6 e2e): 1519 → 1525
```

## References

- [ADR 0155](0155-string-patterns-strategy.md) — long-term pattern matcher strategy.
- [ADR 0112](0112-string-abi-refactor.md) — boxed-string-object layout used by both args.
- [ADR 0103](0103-phase2-7q-stdlib-string.md) — string namespace dispatch precedent.
- [ADR 0063](0063-phase2-6c-tag-locals.md) — TaggedValue Local read trap that the new emit_if branch bypasses for Nil.
- [ADR 0081](0081-phase2-8e-iter-next.md) — multi-return truncation precedent (Next).
- [Lua 5.4 §6.4.1](https://www.lua.org/manual/5.4/manual.html#6.4.1) — pattern semantics target.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M7 milestone.
