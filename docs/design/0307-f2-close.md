# 0307. F2 integer subtype — milestone close (+ i64 bitwise)

- **Status:** Accepted — **closes F2** per ADR 0300's audit scope; generalization breadth pinned as F2-G
- **Kind:** Architecture Decision
- **Date:** 2026-07-09
- **Deciders:** ShortArrow

## This ADR's code change

- ✅ `& | ~` (BitAnd/BitOr/BitXor) join the integer-tree gate and `emit_int_expr` — direct `arith.andi/ori/xori` on i64, exact beyond 2^53 (the legacy path's f64→i64→f64 round-trip saturates big operands before masking).
- ❌ Shifts (`<< >>`) stay on the legacy path — Lua's >=64-shift → 0 and negative-shift-reverses semantics are their own slice (F2-G).

## F2 scorecard (audit scope = ADR 0300)

| Item | Status | ADR |
|---|---|---|
| F2-A..F (math.type / `//` / `1 == 1.0` / tointeger / bitwise / constants) | ✅ pre-existing, probe-verified | 0300 |
| R1 probe: `maxinteger + 1 == mininteger` wraps | ✅ end-to-end `true` | 0304-0306 |
| R1 probe: runtime >2^53 local precision | ✅ exact | 0304-0305 |
| R2: `math.type(10 / 4)` → "float" | ✅ | 0301 |
| R3: `math.type("x")` → nil | ✅ | 0302 |
| Bitwise beyond 2^53 (same disease class, found post-audit) | ✅ `& \| ~` | this ADR |

**Every Lua-semantic deviation identified in the F2 audit is fixed.** F2 is declared closed at the audit scope.

## Coverage boundary (what "closed" means precisely)

The i64 slot machinery activates under the chunk-level eligibility gate (ADR 0304): whole-chunk whitelist of integer-tree stores, comparisons, and plain-arg prints. Outside the gate — function bodies, integer values crossing call/TaggedValue/table boundaries, shifts, the `lumelir run` CLI's arg[]-prologue chunks — programs fall back to the pre-F2 f64 behavior: correct within ±2^53, imprecise beyond, no wraparound. That fallback domain is **F2-G** (generalization):

- F2-G-1: shifts with Lua >=64 / negative-shift semantics
- F2-G-2: fn-body locals (needs Return whitelisting + ret-ABI conversion)
- F2-G-3: boundary conversions (call args, TaggedValue `TAG_INTEGER`, table elements)
- F2-G-4: gate removal (subtype alone decides; requires G-2 + G-3)

F2-G is quality-of-coverage work, not spec-gap closure — scheduled with N8 performance or on demand when real programs hit the boundary.

## Tests

4 e2e (`tests/phase4_f2_close_bitwise.rs`): `maxinteger & 1 == 1` (exact masking of a big operand); `12 | 3`; big xor flips low bit exactly; mixed `(a + 2) & 12` tree. Suite 1841 → 1845.

## References

- ADR 0300 — the audit whose scope this closes.
- ADRs 0301-0306 — the R1/R2/R3 fixes.
- Roadmap 2026-07-03 — F2 was budgeted 6-10 sessions; actual spend ~2.5 sessions after the audit re-scope.
