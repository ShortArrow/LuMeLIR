# 0302. `math.type` accepts any value (F2-R3)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-07-07
- **Deciders:** ShortArrow

## Scope (literal)

- ✅ `Builtin::MathType` param kind changed `[Number]` → `[TaggedValue]` — the ADR 0111 "any" sentinel skips the ADR 0110 static arg-kind gate.
- ✅ Non-number args (String / Bool / Table literals and Locals) flow to the codegen classifier, which returns Unknown for non-Number shapes → nil, matching Lua §6.7.
- ✅ Number classification unchanged (integer / float via ADR 0301 classifier).
- ❌ Side-effectful args (`math.type(f())`) — the classifier answers without evaluating the arg, so a call's side effects don't run. Pre-existing deviation (predates this ADR), noted for the F2-R1 arc.

## Tests

4 e2e (`tests/phase4_f2r3_mathtype_anyvalue.rs`): String nil; Bool nil; Table nil; integer/float still classify. Suite 1820 → 1824.

## References

- ADR 0110 — arg-kind validation gate.
- ADR 0111 — TaggedValue "any" sentinel.
- ADR 0300 — F2 audit that pinned this gap as R3.
- ADR 0301 — the shared classifier.
- Lua 5.4 §6.7 — `math.type`.
