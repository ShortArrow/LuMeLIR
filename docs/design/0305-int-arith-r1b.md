# 0305. i64 arithmetic on int slots (F2-R1-b)

- **Status:** Accepted (implements ADR 0303 §R1-b for `+ - *`)
- **Kind:** Architecture Decision
- **Date:** 2026-07-07
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ Gate widened (`is_gate_int_expr`): store RHS may be a pure integer tree — Integer literal, Integer-subtyped Number local, or `+ - *` BinOp over such trees.
- ✅ New codegen helper `emit_int_expr`: recursive i64 emission (`arith.addi/subi/muli`, i64 loads from int slots, i64 constants). Wraparound is LLVM's natural two's-complement overflow — exactly Lua §3.4.1 semantics.
- ✅ **Both ADR 0300 R1 probes fixed**: `math.maxinteger + 1` wraps to mininteger; `local big = 9007199254740992; big + 1` exact.
- ❌ FloorDiv / Mod — need Lua floor-semantics fixes (`divsi` + sign correction; `remsi` + `+b` correction); gate excludes them, chunks using `//`/`%` keep f64. R1-c.
- ❌ Bitwise on int slots — existing f64→i64→f64 path still runs (correct, just round-trips). R1-c drops the round-trip.
- ❌ Comparisons on int slots (`w == mn` as expressions) — gate excludes comparison stores; print of Bool from int comparison bails the chunk. R1-c.
- ❌ Function bodies / boundaries — R1-c.

## Tests

5 e2e (`tests/phase4_f2r1b_int_arith.rs`): maxinteger+1 wraps; runtime >2^53 add exact; `a * b - 2` tree; mininteger-1 wraps up; `/` chunk bails to f64 (2.5). Suite 1829 → 1834.

## References

- ADR 0303 — design (§R1-b).
- ADR 0304 — R1-a slots.
- Lua 5.4 §3.4.1 — integer arithmetic + wraparound.
