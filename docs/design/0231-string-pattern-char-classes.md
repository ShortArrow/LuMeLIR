# 0231. Pattern Matcher Core — Char Classes + `.` Wildcard

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Fourth (closing) M7 sub-ADR. [ADRs 0228-0230](0228-string-find-plain.md) shipped `string.find` (single + multi return) and `string.match` in literal-byte mode. Real Lua programs use patterns like `%d` (digit), `%a` (letter), `.` (any), `%.` (literal dot). This ADR adds those — a real but bounded pattern matcher — keeping the existing single-/multi-return surface stable.

[ADR 0155](0155-string-patterns-strategy.md)'s long-term plan is a full port of the Lua reference `lstrlib.c` matcher (~600 LOC). This ADR delivers ~80% of real-world use (anchored substring + char-class probe) at ~30% of the port budget. Quantifiers (`*`, `+`, `?`, `-`), sets (`[...]`), anchors (`^`, `$`), captures (`(...)`), and balanced match (`%b()`) remain deferred.

## Scope (literal)

- ✅ New runtime helper `extern "C" fn lumelir_string_match_extents(s_ptr, pat_ptr) -> i64`. Packed return: low 32 bits = 1-indexed start, high 32 bits = matched byte length. 0 = no match.
- ✅ Char classes: `%a` `%A` (alpha), `%d` `%D` (digit), `%s` `%S` (whitespace), `%w` `%W` (alphanumeric), `%p` `%P` (punctuation), `%l` `%L` (lowercase), `%u` `%U` (uppercase), `%x` `%X` (hex digit), `%c` `%C` (control).
- ✅ `.` wildcard matches any single byte.
- ✅ Magic-char escape via `%X` where `X` is any non-class character — `%.` matches literal `.`, `%%` matches literal `%`, etc.
- ✅ Both `string.find` arms (single-return + multi-return) switched to the new helper. Existing ADR 0228 / 0229 / 0230 tests stay green because the new matcher is a strict superset of literal-byte search.
- ✅ Multi-return `end` now derives from the matched byte length (not pattern byte length) so `string.find("abc1", "%a%d")` correctly returns `(3, 4)`.
- ❌ Quantifiers `*`, `+`, `?`, `-`. Future sub-ADR.
- ❌ Sets `[abc]`, ranges `[a-z]`, complement `[^...]`. Future sub-ADR.
- ❌ Anchors `^` (string start), `$` (string end). Future sub-ADR.
- ❌ Captures `(...)` returning extra String values. Future sub-ADR.
- ❌ Balanced match `%b()`. Future sub-ADR.
- ❌ `%f[set]` frontier match. Future sub-ADR.
- ❌ `string.match` pattern-aware path. Plain mode still returns `sub`-on-hit (incorrect for patterns); upgrading match requires allocating a new boxed String of the matched bytes via the GC allocator. Future sub-ADR.

## Decision

### Packed return ABI

```
[ high 32: matched_byte_length | low 32: 1-indexed_start ]
```

- 0 = no match (both halves zero).
- Decode in MLIR: `start = packed & 0xFFFFFFFF`, `len = (packed >>> 32) & 0xFFFFFFFF`, `end = start + len - 1`.
- Fits the x86_64 SystemV single-i64-return ABI; no struct return needed.
- 2^32 = 4 GiB limit on string size, which is below the ADR 0184 4 GiB GC payload cap. Safe.

### Matcher algorithm

`try_match_at(s, start, pat)` walks `pat` left-to-right against `s[start..]`:

```
while p_i < pat.len:
    if s_i >= s.len: return None
    case pat[p_i]:
        b'%' -> consume `%X` pair, check matches_class(s[s_i], X)
        b'.' -> always matches one byte
        c    -> match s[s_i] == c
    s_i += 1; p_i += (1 or 2)
return Some(s_i - start)
```

Outer loop tries each `start ∈ 0..=s_len` and returns the first match. `O(N * M)` where `N=s_len`, `M=pat_len`. Sufficient for typical short patterns; quantifiers will add a backtracking inner loop.

### Why upgrade find but not match

`string.find` returns positions — the matched byte length is already part of the multi-return shape (end position). Switching is a strict supertyping of the literal path.

`string.match` returns the matched substring. In plain mode that's `sub` (the input pattern itself, which is also the matched bytes). In pattern mode that's the bytes of `s` at positions `[start, end]` — generally NOT equal to `sub`. To produce them we must allocate a new boxed String — a non-trivial codegen change (allocator integration) deferred to the next M7 sub-ADR.

This sub-ADR explicitly documents the limitation: `string.match` with magic-char patterns continues to return `sub` itself, which is observable as incorrect when the user expects e.g. the actual digit string for `string.match(s, "%d")`. The test `find_digit_class_position` covers find; no match test asserts the pattern-mode match-bytes behaviour.

## Tests

`tests/phase4_string_find_pattern.rs` (NEW, 7 e2e):

1. `string.find("abc1def", "%d")` → `4`.
2. `string.find("abc123", "%a")` → `1` (alpha at start).
3. `string.find("foo axb bar", "a.b")` → `5` (`.` wildcard matches `x`).
4. `string.find("file.lua", "%.lua")` → `5` (escaped dot matches literal).
5. `local s, e = string.find("ab1c", "%a%d")` → `(2, 3)` — start past the leading 'a' since `%a%d` requires the alpha+digit pair.
6. `string.find("abcdef", "%d")` → `nil` (no digit in pure-alpha s).
7. `string.find("123x4", "%D")` → `4` — complement class finds the non-digit.

All ADR 0228 / 0229 / 0230 e2e remain green: literal-byte patterns route through the same helper (no `%` or `.` in pat → falls into the literal arm of the matcher).

## Test count delta

```
Step 0:  1533 (after ADR 0230)
C3 (impl + 7 e2e): 1533 → 1540
```

## M7 milestone close

ADRs 0228 + 0229 + 0230 + 0231 land the M7 minimum-viable close at 4/4-6 sub-ADRs. Remaining M7-stretch work:

- **string.match pattern-aware return** — allocate new boxed String of matched bytes.
- **Quantifiers** `*` `+` `?` `-`.
- **Sets** `[...]` + ranges `[a-z]` + complement `[^...]`.
- **Anchors** `^` `$`.
- **Captures** `(...)` → extra return values.
- **`string.gsub`** substitution.
- **`string.gmatch`** iterator (composes with ADR 0085 generic-for).
- **`init` / `plain` args**.
- **Balanced match `%b()`** + **`%f[set]` frontier**.

## References

- [ADR 0155](0155-string-patterns-strategy.md) — long-term pattern matcher strategy.
- [ADR 0228](0228-string-find-plain.md) — single-return literal `string.find`.
- [ADR 0229](0229-string-find-multireturn.md) — multi-return literal `string.find`.
- [ADR 0230](0230-string-match-plain.md) — literal `string.match`.
- [ADR 0112](0112-string-abi-refactor.md) — boxed-string layout consumed by the matcher.
- [ADR 0184](0184-gc-type-meta-size-guard.md) — 4 GiB GC payload cap (fits the 32-bit length pack).
- [Lua 5.4 §6.4.1](https://www.lua.org/manual/5.4/manual.html#6.4.1) — pattern semantics.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M7 milestone close.
