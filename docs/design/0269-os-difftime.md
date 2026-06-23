# 0269. `os.difftime(t2, t1)` (N7-8)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-23
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ New `Builtin::OsDifftime`; arity `(2, 2)`; param `[Number, Number]`; ret `[Number]`.
- ✅ Codegen: inline `arith.subf t2, t1`. No libc dependency — `difftime` is just subtraction when timestamps are doubles.

## Tests

3 e2e: `(100, 60) == 40`, `(5, 10) == -5`, `(7, 7) == 0`. 1672 → 1675.

## References

- Lua 5.4 §6.9.
- ADR 0264-0267 — sibling os.* increments.
