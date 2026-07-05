# 0301. `math.type` over BinOp results (F2-R2)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-07-06
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ ADR 0234's nested BinOp subtype classifier extracted into `pub fn classify_number_subtype(expr, locals)` in `src/hir/mod.rs`. Rules unchanged (Lua §3.4.1): `/` and `^` always Float; integer-preserving ops (`+ - * // %` and bitwise) yield Integer iff both operands do; any Float operand → Float; else Unknown.
- ✅ `math.type` codegen arm replaces its 5-arm shape match (literal Integer / literal Number / Local-Integer / Local-Float / fallthrough-nil) with one call to the shared classifier — BinOp arguments now classify instead of falling to nil.
- ✅ `propagate_number_subtype` delegates to the extracted fn (`use ... as classify`), so both consumers stay in lock-step by construction.
- ❌ Call-result subtypes (`math.type(f())`) — classifier returns Unknown for Call; needs ret-subtype tracking on `HirFunction`. Deferred.
- ❌ UnaryOp (`math.type(-x)`) — returns Unknown; small follow-up.

## Fixed vs ADR 0300 audit

| Probe | Before | After |
|---|---|---|
| `math.type(10 / 4)` | `nil` | `float` ✅ |
| `math.type(2^2)` | `nil` | `float` ✅ |
| `math.type(1 + 2.0)` | `nil` | `float` ✅ |
| `math.type(x * 2)` (x integer Local) | `nil` | `integer` ✅ |

## Tests

6 e2e (`tests/phase4_f2r2_mathtype_binop.rs`): div float; pow float; int+int integer; int+float float; floordiv integer; Local-through-BinOp. Suite 1814 → 1820.

## References

- ADR 0234 — the classifier's origin (Local propagation pass).
- ADR 0300 — F2 audit that pinned this gap as R2.
- Lua 5.4 §3.4.1.
