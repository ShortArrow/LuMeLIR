# 0181. Function Parameter String-Context Inference

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-09
- **Deciders:** ShortArrow

## Context

[ADR 0180](0180-param-table-context-inference.md) extended `infer_param_kinds` to detect Table-context uses of a parameter and refine its kind. The same body-walker still leaves String-context uses unrefined — parameters used as the operand of `..` (Concat) or as the first arg of `string.*` methods stay at the `Number` default, breaking:

```lua
local function append_x(s) return s .. "x" end    -- call-site Local arg rejected as Number != String
local function up(s)       return string.upper(s) end  -- string.upper arg expected String, got Number
```

ADR 0180's pattern (single body-walk, last-write-wins, body-decisive merge) generalises directly to String — each kind is one set of marking arms.

## Scope (literal)

- ✅ `BinOp { Concat, lhs=Ident(param), .. }` and `{ rhs=Ident(param), .. }` — `param .. x` or `x .. param`.
- ✅ `Call { callee=Index{ Ident("string"), Str(method) }, args=[Ident(param), ...] }` — `string.upper(param)`, `string.len(param)`, `string.sub(param, ...)`, etc.
- ✅ Body/external merge at `LowerCtx::for_function`: String joins Function + Table in the "body wins" arm.
- ✅ Conflict policy: last-write-wins (consistent with 0180).
- ❌ Bool-context inference — `if param`, `not param`, `param and x`, `param or x` all permit any kind via Lua's truthiness (`false`/`nil` are falsy, everything else truthy). Inferring Bool from these would be incorrect.
- ❌ Number-context inference — Number is the existing default; no marking needed.
- ❌ String detection via `tostring(param)` / `print(param)` — both accept any kind, give no String signal.
- ❌ Cross-procedure inference.

## Decision

### HIR side (`src/hir/mod.rs`)

In `infer_param_kinds`'s body walker:

1. **New helper `mark_ident_as_string`** — direct mirror of 0180's `mark_ident_as_table`, sets `ValueKind::String`.

2. **BinOp Concat arm extension**:
   ```rust
   ExprKind::BinOp { op: BinOp::Concat, lhs, rhs } => {
       mark_ident_as_string(lhs, name_to_idx, kinds);
       mark_ident_as_string(rhs, name_to_idx, kinds);
       visit_expr(lhs, name_to_idx, kinds);
       visit_expr(rhs, name_to_idx, kinds);
   }
   ```
   The existing generic `BinOp` arm recurses without marking; this new arm intercepts only Concat.

3. **Call to `string.*` method**: extend the existing Call arm — when the callee is `Index { target: Ident("string"), key: Str(_) }` (Index-callee form lowered later by ADR 0091 but visible at AST), mark args[0] as String.

4. **Merge extension at `LowerCtx::for_function`** (line ~2052):
   ```rust
   let kind = match body_kinds[i] {
       ValueKind::Function(_) | ValueKind::Table | ValueKind::String => body_kinds[i],
       _ => external_kinds[i],
   };
   ```

### Tests

`tests/phase2_6plus_param_string_inference.rs` (NEW, ~3 e2e):
1. `param .. "x"` Concat operand.
2. `"x" .. param` Concat operand (rhs side).
3. `string.upper(param)` Index-callee Call arg[0].

## Alternatives considered

- **Generalise the body-walker into a kind-parameterised helper** before adding more kinds. Rejected for this ADR — would retroactively touch ADR 0180; better as a future Tidy First refactor once 2-3 kinds exist. Deferred.
- **Include Bool inference** via `if param`. Rejected — Lua truthiness means `if any_kind` is legal; would produce false positives.
- **Detect `param[i]` (subscript) as String inference** since strings can be indexed. Rejected — Lua actually rejects `s[i]` for strings; you need `string.sub(s, i, i)` or `s:sub(i, i)`. The Index→Table rule (ADR 0180) is correct.

## Consequences

**Positive**
- Idiomatic `string`-method patterns work with locally-bound String args (not just literals).
- Concat-only helpers (`local function fmt(s) return "[" .. s .. "]" end`) compile cleanly.
- Pattern symmetric to 0180; future ADRs can keep extending.

**Negative**
- A param used as both Concat operand AND Function callee gets last-write-wins (rare in practice).
- Body-walker grows two arms; surface increase remains small.

**Locked in until superseded**
- `BinOp::Concat` + `string.*` Index-callee enumeration is the entirety of String-context detection in this ADR. Adding signals (e.g. `s:upper()` MethodCall form) is a follow-up.

## Documentation updates

- [x] §8 — adds 0181.
- [x] ADR 0180 reference — 0181 extends the same pattern for String.

## Test count delta

```
Step 0: 1404 (after ADR 0180)
C1 (doc): 1404 → 1404
C2 (3 e2e Red Day 0): 1404 → 1404
C3 (HIR impl): 1404 → 1407
```

## Critical files

- `src/hir/mod.rs`:
  - Add `mark_ident_as_string` helper.
  - Extend body-walker with BinOp::Concat arm + `string.*` Call detection.
  - Extend body/external merge to include String in "body wins".
- `tests/phase2_6plus_param_string_inference.rs` (NEW) — 3 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| Concat detection fires on `param .. param` where param is actually expected as Number | Concat semantically requires String (or Number with implicit `tostring`). Marking as String is the documented Lua semantic; if a user wants Number-arithmetic on the same name they should rename. |
| MethodCall form `s:upper()` not detected | Documented as follow-up; the user can switch to `string.upper(s)` form or pass an upvalue. |
| External call-site overwrites body-walk String | Body/external merge extended in the same commit; body String wins. |
| Param used as both Function callee and Concat operand | Last-write-wins, consistent with 0180. |

## Future work

- MethodCall form `s:upper()` String inference (probable next ADR — small, single arm).
- Kind-parameterised body-walker helper (Tidy First refactor after ≥3 kinds).
- IndexAssign value-side non-Local TaggedValue (orthogonal, was the original 0181 candidate; deferred after investigation surfaced multi-branch codegen complexity).

## References

- [ADR 0180](0180-param-table-context-inference.md) — direct pattern precedent.
- [ADR 0091](0091-phase2-6plus-callee-norm.md) — Index-callee normalisation (string.* methods lower through this).
- Lua 5.4 reference §3.4.6 — concatenation operator semantics.
