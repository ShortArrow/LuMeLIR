# 0230. `string.match` Literal Substring Match (Plain Mode)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Third M7 sub-ADR. [ADR 0228](0228-string-find-plain.md) shipped `string.find` plain mode; [ADR 0229](0229-string-find-multireturn.md) widened it to multi-return. `string.match` is the sibling: same input shape, different return convention — returns the matched substring (and any captures) instead of positions.

In plain (literal) mode the matched substring equals `sub` itself, so this sub-ADR adds zero new runtime code: the emit arm reuses `lumelir_string_find_plain` for the hit/miss decision, then stores `sub` itself as a String on hit or Nil on miss into a tmp TaggedValue slot.

## Scope (literal)

- ✅ New `Builtin::StringMatch` HIR variant. `string_from_method("match") → Some(StringMatch)`. Name `"string.match"`, arity `(2, 2)`, `ret_kinds = [TaggedValue]`, `param_kinds_for_arity = [String, String]`. `infer_kind → TaggedValue`.
- ✅ Codegen emit arm reuses `lumelir_string_find_plain` for the hit/miss decision; on hit stores `TAG_STRING + sub_ptr` into the tmp TaggedValue slot, else stores `TAG_NIL`.
- ✅ Composes with the ADR 0228 non-trapping `if Local(TaggedValue)` cond path — `if string.match(s, sub) then ... end` works.
- ✅ Reuses the existing `emit_value_slot_store_string` / `emit_value_slot_store_nil` chokepoints — no new tagged-slot writer.
- ❌ Magic-char patterns. Same scope literal as ADR 0228 §"❌ Pattern magic chars". Patterns with `%d`, `%a`, `.`, `*`, `+`, `[`, `]`, `(`, `)`, `^`, `$` silently no-match.
- ❌ Captures `(...)` returning extra String values. Pattern matcher port owns these.
- ❌ `init` start-position arg. Future sub-ADR.
- ❌ Multi-return for any future pattern with captures. ADR 0021 / 0081 multi-assign builtin dispatch will land alongside captures.

## Decision

### Reuse vs new helper

The Rust side stays exactly as ADR 0228 left it. `lumelir_string_find_plain` returns the 1-indexed start position; whether the caller wants that position (`find`) or the substring (`match`), the bytes for the answer are already in-hand:

- `find` builds `TAG_NUMBER + f64(start_position)`.
- `match` builds `TAG_STRING + sub_ptr`.

Both branches dispatch on the same `pos != 0` boolean. The emit arms are siblings.

### Why ship match plain before captures

Most real `string.match` use is the "did this substring occur, and what was the match" pattern:

```lua
local ext = string.match(name, ".lua")
if ext then ... end
```

In plain mode the answer is always `sub` itself, but the value still flows through downstream code that doesn't care about pattern semantics — concat, print, length, comparison. Shipping the plain mode now lets that code compile and run; the pattern matcher port replaces the literal-substring lookup with real matching later, and the surface stays stable.

## Tests

`tests/phase4_string_match_plain.rs` (NEW, 4 e2e):

1. `string.match("hello world", "world")` → `"world"`.
2. `string.match("hello", "xyz")` → `nil`.
3. `if string.match(name, ".lua") then ...` matched path.
4. Same `if` shape over the no-match case (proves the ADR 0228 non-trapping cond path also covers `string.match`).

## Test count delta

```
Step 0:  1529 (after ADR 0229)
C3 (impl + 4 e2e): 1529 → 1533
```

## References

- [ADR 0228](0228-string-find-plain.md) — `string.find` plain foundation; runtime helper + non-trapping cond path reused here.
- [ADR 0229](0229-string-find-multireturn.md) — `string.find` multi-return sibling.
- [ADR 0155](0155-string-patterns-strategy.md) — long-term pattern matcher strategy.
- [Lua 5.4 §6.4.1](https://www.lua.org/manual/5.4/manual.html#6.4.1) — `string.match` return spec.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M7 milestone.
