# 0300. Integer subtype (F2) status audit + residual gaps

- **Status:** Accepted (audit; residual implementation split into F2-R1..R3)
- **Kind:** Architecture Decision
- **Date:** 2026-07-06
- **Deciders:** ShortArrow

## Why this ADR exists

Roadmap [2026-07-03](../notes/roadmap-2026-07-03-lua54-completion.md) budgeted F2 (integer subtype) at **6-10 sessions**, quoting the stale conformance-doc claim "number (subtype: integer) ❌ — LuMeLIR has only f64". A probe audit (below) shows ADRs 0209-0214 (integer literal arc) + 0232 (M8-A `NumberSubtype`) + M11 sweeps already cover most of the listed scope. This ADR records the probe results and re-scopes the residual to ~2-3 sessions.

## Probe results (all verified on commit `70d3028`)

| Probe | Lua 5.4 | LuMeLIR | Verdict |
|---|---|---|---|
| `math.type(1)` | `integer` | `integer` | ✅ |
| `math.type(1.5)` | `float` | `float` | ✅ |
| `7 // 2` | `3` | `3` | ✅ F2-C done |
| `1 == 1.0` | `true` | `true` | ✅ F2-F done |
| `math.tointeger(3.0)` | `3` | `3` | ✅ F2-E done |
| `5 & 3`, `1 << 4` | `1`, `16` | `1`, `16` | ✅ F2-D done |
| `math.maxinteger` / `mininteger` / `pi` / `huge` | exact | exact | ✅ constants done |
| `print(1 + 2)` | `3` | `3` | ✅ integer display |
| `9007199254740993` literal | exact | exact | ✅ i64 literal path |
| `9007199254740992 + 1` (static fold) | exact | exact | ✅ |
| `math.type(7 // 2)` | `integer` | `integer` | ✅ |
| **`math.maxinteger + 1 == math.mininteger`** | `true` (wraps) | `false` | ❌ gap R1 |
| **`local big = 2^53; big + 1` (runtime slot)** | exact | `9.0072e+15` | ❌ gap R1 |
| **`math.type(10 / 4)`** | `float` | `nil` | ❌ gap R2 |
| **`math.type(2^2)`** | `float` | `nil` | ❌ gap R2 |
| `math.type("x")` | `nil` (any value ok) | HIR arg-kind error | ❌ gap R3 |

## Residual decomposition

### F2-R1 — i64 runtime slots for Integer-subtyped locals (1-2 sessions, hir-refactor + codegen)

The deep one. Static folds preserve i64 precision but a Local slot is f64; runtime arithmetic on integer values > 2^53 loses precision, and overflow wraparound (`maxinteger + 1 → mininteger`) is impossible on f64. Options preserved from ADR 0209's deferral:
- (a) `ValueKind::Integer` first-class variant — the honest fix; ~57-site `ValueKind::Number` consumer updates.
- (b) i64-in-f64-slot NaN-boxing — smaller diff, deeper cleverness.
Recommend (a) attempted as its own arc with the same sub-step discipline as F1-C.

### F2-R2 — subtype propagation through BinOp results (0.5-1 session, hir-arm)

`math.type` returns nil for expression results because `NumberSubtype` doesn't flow through BinOp lowering. Rules per Lua 5.4 §3.4.1: `/` and `^` always Float; `+ - * // %` are Integer iff both operands Integer; bitwise always Integer. Pure `infer`-side change + MathType consumer.

### F2-R3 — `math.type` accepts any value (0.25 session, hir-arm)

Lua spec: `math.type(x)` returns nil for non-numbers rather than erroring. Our ADR 0110 arg-kind gate rejects String args at HIR. Relax the gate for MathType specifically (TaggedValue accept + runtime tag dispatch already exists for the nil case).

## Consequence for the completion roadmap

F2's 6-10 session line item collapses to **2-3.5 sessions** (R1 dominant). The grand total in roadmap 2026-07-03 (55-84 Option B) reduces by ~4-6 sessions.

## References

- ADRs 0209-0214 — integer literal arc.
- ADR 0232 — M8-A NumberSubtype.
- Roadmap 2026-07-03 — F2 original estimate (superseded by this audit).
- Lua 5.4 §3.4.1 — arithmetic subtype rules.
