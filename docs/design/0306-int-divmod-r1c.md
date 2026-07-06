# 0306. i64 floor-div / mod / comparisons (F2-R1-c steps 1-2)

- **Status:** Accepted (steps 1-2; fn bodies / boundaries / gate removal remain)
- **Kind:** Architecture Decision
- **Date:** 2026-07-07
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ Gate (`is_gate_int_expr`) admits `//` and `%` in integer trees.
- ✅ `emit_int_expr` emits Lua floor semantics (§3.4.1) branch-free:
  - `//`: `q = divsi(l, r)` then `q -= 1` when `rem != 0 && (rem ^ r) < 0` (select).
  - `%`: `rem = remsi(l, r)` then `rem += r` under the same condition — result sign follows the divisor.
- ✅ Zero divisor traps via `emit_trap_if` with new diagnostic global `s_int_div_zero` ("integer division by zero"), matching Lua's error behavior for `n//0` / `n%0` on integers.
- ✅ **Step 2 — comparisons**: gate admits `== ~= < <= > >=` over integer trees; codegen stores Bool from `arith.cmpi` on i64 operands. The check is keyed on `int_slot` flags (`int_tree_on_slots`), NOT subtype — a gate-bailed chunk with Integer-subtyped f64 locals must not divert here.
- ✅ **ADR 0300 headline probe closed end-to-end**: `math.maxinteger + 1 == math.mininteger` → `true`.
- ❌ Bitwise on int slots (current f64 round-trip stays; correct, just indirect) — R1-c step 3.
- ❌ Function-body locals, boundary conversions, gate removal — R1-c step 3 (final).

## Tests

7 e2e (`tests/phase4_f2r1c_int_divmod.rs`): `-7 // 2 == -4` (floors, not truncates); `7 // 2 == 3`; `-7 % 2 == 1` (sign follows divisor); `7 % -2 == -1`; `n // 0` traps; wraparound equality end-to-end (`true`); `<` beyond 2^53 exact. Suite 1834 → 1841.

## References

- ADR 0303 — R1 design.
- ADR 0305 — R1-b (`+ - *`).
- Lua 5.4 §3.4.1 — floor division, modulo, and the zero-divisor error.
