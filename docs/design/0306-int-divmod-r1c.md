# 0306. i64 floor-div / mod + zero trap (F2-R1-c step 1)

- **Status:** Accepted (first R1-c slice; comparisons / fn bodies / boundaries / gate removal remain)
- **Kind:** Architecture Decision
- **Date:** 2026-07-07
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ Gate (`is_gate_int_expr`) admits `//` and `%` in integer trees.
- ✅ `emit_int_expr` emits Lua floor semantics (§3.4.1) branch-free:
  - `//`: `q = divsi(l, r)` then `q -= 1` when `rem != 0 && (rem ^ r) < 0` (select).
  - `%`: `rem = remsi(l, r)` then `rem += r` under the same condition — result sign follows the divisor.
- ✅ Zero divisor traps via `emit_trap_if` with new diagnostic global `s_int_div_zero` ("integer division by zero"), matching Lua's error behavior for `n//0` / `n%0` on integers.
- ❌ Comparisons on int slots — R1-c step 2.
- ❌ Bitwise on int slots (current f64 round-trip stays; correct, just indirect) — R1-c step 2.
- ❌ Function-body locals, boundary conversions, gate removal — R1-c step 3 (final).

## Tests

5 e2e (`tests/phase4_f2r1c_int_divmod.rs`): `-7 // 2 == -4` (floors, not truncates); `7 // 2 == 3`; `-7 % 2 == 1` (sign follows divisor); `7 % -2 == -1`; `n // 0` traps with the right message. Suite 1834 → 1839.

## References

- ADR 0303 — R1 design.
- ADR 0305 — R1-b (`+ - *`).
- Lua 5.4 §3.4.1 — floor division, modulo, and the zero-divisor error.
