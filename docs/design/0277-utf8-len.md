# 0277. `utf8.len(s)` codepoint count (N7-16)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-26
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ New `Builtin::Utf8Len`; arity `(1, 1)`; param `[String]`; ret `[Number]`.
- ✅ Codegen: `scf.while` over the byte buffer; increments the count by 1 for every byte whose top 2 bits ≠ `0b10` (non-continuation byte).
- ✅ New `utf8.*` namespace dispatch via `Builtin::utf8_from_method` and `from_namespace_method` arm.
- ❌ Optional `i`/`j` range args — deferred. Currently always counts the whole string.
- ❌ Invalid-UTF-8 detection (Lua spec returns `(nil, position)` on malformed input) — deferred. Current impl counts non-continuation bytes only, so malformed sequences are silently accepted.
- ❌ Other `utf8.*` (char/codepoint/codes/offset) — deferred to follow-up sub-ADRs.

## Tests

3 e2e (`tests/phase4_n7_utf8_len.rs`): ASCII (5 → 5); 2-byte multibyte (`"héllo"` → 5 codepoints / 6 bytes); empty string → 0. 1694 → 1697.

## References

- Lua 5.4 §6.5.
- ADRs 0262-0276 — sibling N7 increments.
