# 0233. `print(Local-Integer)` + `tostring(Local-Integer)` Precision

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-21
- **Deciders:** ShortArrow

## Context

Second M8 sub-ADR. [ADR 0232](0232-local-number-subtype.md) lit up `LocalInfo::subtype` and consumed it from `math.type`. This ADR adds the next two consumers: `print(Local-Integer)` and `tostring(Local-Integer)` route through a `%lld` formatting path so precision survives up to the f64-representable integer range (±2^53).

[ADR 0214](0214-print-integer-subtype.md) shipped the equivalent path for `HirExprKind::Integer` literals. This ADR extends the same logic to Locals whose subtype has been resolved to `Integer` by the M8-A post-pass.

## Scope (literal)

- ✅ Print Integer fast path widens: in addition to `HirExprKind::Integer(_)`, the Print emit arm now recognises `HirExprKind::Local(idx)` where `locals[idx].kind == Number` AND `locals[idx].subtype == NumberSubtype::Integer`. The branch loads the slot's f64, `fptosi`s to i64, and prints via the `fmt_lld_raw` (`"%lld\0"`) format.
- ✅ ToString fast path: same Local-Integer detection at the top of the `Callee::Builtin(Builtin::ToString)` arm, before the existing TaggedValue / Index / Call dispatches. Delegates to a new helper `emit_tostring_integer` that mirrors the Number snprintf path with `%lld` + a 32-byte scratch buf (covers any i64 sign + decimal + NUL).
- ✅ Float Locals continue to use the `%g` path — the dispatch is `Integer-only`.
- ✅ Integer literal print/tostring (ADR 0214 / ADR 0026) keeps its existing fast path; this ADR adds the Local-resolved version as a strict superset.
- ❌ Precision for integers outside ±2^53. The slot still holds f64 (Phase B); values that lose mantissa precision at LocalInit time stay imprecise on the print side. Phase C i64 slots (future M8 sub-ADR) lift this ceiling.
- ❌ `io.write(Local-Integer)`. Sibling consumer; mechanical extension; future sub-ADR.
- ❌ String concat where one operand is a Local-Integer. The concat code path does its own tostring-equivalent for Number operands; widening to subtype-aware concat is a separate sub-ADR.
- ❌ String.format `%d` / `%i` with Local-Integer arg. Future sub-ADR.

## Decision

### Print arm dispatch (cheapest first)

```text
1. HirExprKind::Integer(i)         → %lld with const i64
2. Local + Number + Integer subtype → load f64, fptosi i64, %lld   (NEW)
3. Local + TaggedValue              → tagged dispatch
4. Index / Call(...) returning TaggedValue → inline materialise
5. fall-through: emit_expr + emit_print_value_raw                 (%g for Number)
```

The new arm slots between the literal-Integer arm and the TaggedValue arm so it benefits the common case while keeping all later arms unchanged.

### ToString arm

```text
1. Local + Number + Integer subtype → emit_tostring_integer(load f64) (NEW)
2. TaggedValue Local                → emit_tostring_tagged_local
3. Index { target, key }            → inline tagged materialise
4. Call(...) TaggedValue            → inline tagged materialise
5. Other                            → emit_tostring (the existing %.14g path)
```

### `emit_tostring_integer` helper

```mlir
%i64  = arith.fptosi %f64 : i64
%buf  = emit_gc_alloc 32_bytes SCRATCH_BUF
%fmt  = addressof @fmt_lld_raw
call snprintf(%buf, 32, %fmt, %i64) : i32
%len  = strlen(%buf)
yield emit_string_obj_from_bytes(%buf, %len)
```

Sibling of the Number arm in `emit_tostring` — same alloc + snprintf + boxed-string-from-bytes shape, swapping the format string and the input type.

## Tests

`tests/phase4_m8_local_integer_print.rs` (NEW, 5 e2e):

1. `local i = 42; print(i)` → `"42"`.
2. `local big = 1000000000000; print(big)` → `"1000000000000"` (would print `"1e+12"` via `%g`).
3. `local n = 12345; print(tostring(n))` → `"12345"`.
4. `local n = 99; print("n=" .. tostring(n))` → `"n=99"` (concat round-trip).
5. `local f = 3.5; print(f)` → `"3.5"` (Float subtype keeps `%g`).

## Test count delta

```
Step 0:  1545 (after ADR 0232)
C3 (impl + 5 e2e): 1545 → 1550
```

## References

- [ADR 0232](0232-local-number-subtype.md) — `LocalInfo::subtype` runtime tracking that gates this dispatch.
- [ADR 0214](0214-print-integer-subtype.md) — Integer literal print precision; this ADR generalises to Locals.
- [ADR 0026](0026-phase2-7c-tostring.md) — `emit_tostring` Number arm sibling.
- [ADR 0112](0112-string-abi-refactor.md) — boxed-string layout the new helper produces.
- [`docs/notes/roadmap-status-2026-06-20.md`](../notes/roadmap-status-2026-06-20.md) — M8 milestone.
