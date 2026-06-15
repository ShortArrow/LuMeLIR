# 0196. Number Subtype — Integer / Float Split Design Entry

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-15
- **Deciders:** ShortArrow

## Context

The [Lua 5.4 conformance roadmap](../notes/lua54-conformance-roadmap.md) (2026-06-15) §Conformance gap summary §Priority A ranks **Integer/Float subtype** as the single largest language-level gap. The roadmap explicit recommendation: "start with Integer/Float subtype — biggest single language-level gap, retrofit pressure on bitwise / math / comparison / metamethods grows the longer it waits."

Lua 5.4 spec §2.1 defines `number` as a type with two subtypes:

> Lua has two number subtypes: integer and float. Standard Lua uses 64-bit integers and double-precision (64-bit) floats, but you can also compile Lua so that it uses 32-bit integers and/or single-precision (32-bit) floats. The option with 32 bits for both integers and floats is particularly attractive for small machines and embedded systems.

Current LuMeLIR has only `ValueKind::Number` representing an f64. Every numeric literal, arithmetic op, comparison, and metamethod dispatcher uses f64 throughout. The spec requires:

- **`math.type(x)`** returning `"integer"` / `"float"` / `nil`
- **`//` (floor div) integer result** when both operands are integer; float result if either is float
- **`/` always float** even with integer operands
- **Bitwise ops (`~`, `&`, `|`, `<<`, `>>`)** require integer subtype (or convertible-to-integer via floor); float operand triggers `tointeger` coercion or error
- **`1 == 1.0` is `true`** (cross-subtype equality at value level)
- **`tostring(1)` → `"1"`** vs **`tostring(1.0)` → `"1.0"`** (subtype-distinguished printing)
- **`math.tointeger(x)`** returning integer if `x` is exactly representable
- **Integer overflow** wraps modulo 2^64 (Lua spec); float overflow → `±math.huge`

This is a project-wide redesign that touches HIR, codegen, runtime, tagged-value layout, metamethod dispatch, and every numeric stdlib function. This ADR is the **design entry** — pins the strategy and surfaces the sub-ADR decomposition. Implementation lands across ADRs 0197+.

## Decision

### Strategy — additive subtype split

Introduce a new `ValueKind` variant `Integer` alongside the existing `Number` (renamed conceptually to `Float`; the existing `Number` identifier can stay as the float subtype for source-stability reasons — to be decided in the rename ADR).

- **Static-kind paths** (`ValueKind::Integer`, `ValueKind::Number`) carry the subtype explicitly. Codegen emits i64 ops for Integer paths, f64 ops for Number/Float paths.
- **TaggedValue paths** carry the subtype at runtime via a new tag value. Today `TAG_NUMBER` covers all numbers; the new layout splits to `TAG_INTEGER` / `TAG_FLOAT`. The 16-byte tagged-slot payload remains 8 bytes — interpreted as i64 vs f64 based on tag.
- **Mixed-subtype operations** follow Lua 5.4 §3.4.1 rules: arithmetic coerces to float if either operand is float; integer-only when both are integer (except `/` always float, `//` follows operand types).

### Sub-ADR decomposition

| Sub-ADR | Scope | Estimated session |
|---|---|---|
| 0197 | Parser + lexer: distinguish integer literals (`42`, `0xFF`) from float literals (`42.0`, `1e5`, `0x1.8p3`) | 1 |
| 0198 | HIR: `ValueKind::Integer` variant + `infer_kind` for numeric literals + arithmetic op result kind rules per Lua §3.4.1 | 1-2 |
| 0199 | Codegen: i64 paths for static-kind Integer (arithmetic, comparison, print) | 2-3 |
| 0200 | TaggedValue layout: `TAG_INTEGER` / `TAG_FLOAT` split; payload interpretation per tag | 1-2 |
| 0201 | Floor div `//` and division `/` per Lua §3.4.1 (cross-subtype rules) | 1 |
| 0202 | Bitwise ops require integer (`emit_arith_string_coerce` analog for tointeger coercion) | 1 |
| 0203 | Cross-subtype equality `1 == 1.0` (value-level comparison after tag check) | 1 |
| 0204 | `tostring` subtype-distinguished formatting (`"1"` vs `"1.0"`) | 0.5 |
| 0205 | `math.type` / `math.tointeger` / `math.floor` (integer-returning) | 1 |
| 0206 | Metamethod dispatch: `__add` / `__sub` / etc. handle integer + float promotion | 1-2 |

**Total**: 10 sub-ADRs, 10-15 sessions.

### Migration strategy

The codebase will pass through a partial state where some paths are integer-aware and others still treat all numbers as float. To avoid breakage:

1. **Phase A — additive**: Add `ValueKind::Integer` + `TAG_INTEGER` without changing any existing path's behaviour. Integer literals parse as Number/Float for backward compat. (ADRs 0197 + 0198 entry stubs.)
2. **Phase B — opt-in**: Switch integer literals to parse as `Integer`. Codegen falls back to f64 ops when an Integer path is missing (silent demotion); existing tests stay green because f64 arithmetic on values like `1` produces correct results for the float subset of cases. (ADRs 0199-0202.)
3. **Phase C — strict**: Lift the silent demotion. Integer paths emit i64; mixed ops promote per Lua §3.4.1; tests that rely on f64 semantics for integer-literal operands get updated. (ADRs 0203-0206.)

Phase B is the long-duration phase — it's the bridge state. Codex 第3原則 (anti-ad-hoc) requires that each Phase B sub-ADR documents its silent-demotion fallback and the trigger to lift it.

### `print` output format compatibility

Current LuMeLIR prints integer-valued floats as integers (`print(1+2)` → `"3"`, `print(2.5+3.5)` → `"6"`). After the subtype split:

- `print(1 + 2)` (integer-arithmetic) → `"3"` (unchanged)
- `print(1.0 + 2.0)` (float-arithmetic, integer-valued result) → `"3.0"` (currently outputs `"3"`; will change to `"3.0"` to match Lua spec)
- `print(2.5 + 3.5)` (float, integer-valued result) → `"6.0"` (currently `"6"`; will change)

Existing tests that assert `"6"` for integer-valued float results will need updating. Most existing tests use integer literals (`print(1+2)` → `"3"`) so the breakage surface is smaller than it looks; the affected tests are explicitly fractional-arithmetic ones (Bridge MVP test 1, math tests).

This is a **deliberate compat break** for Lua spec conformance. ADR 0204 records the test migrations.

### Open questions deferred to sub-ADRs

- **Should `ValueKind::Number` be renamed to `ValueKind::Float`?** Pure naming; affects every `infer_kind` arm. ADR 0198 decides.
- **Is `TAG_NUMBER` retired or aliased to `TAG_FLOAT`?** ABI question. ADR 0200 decides; aliasing minimises churn.
- **Integer division by zero**: Lua 5.4 raises a runtime error; how do we surface it in MLIR? Trap with `s_int_div_zero` global. ADR 0201 records.
- **Bitwise float operand**: Lua 5.4 §3.4.2: float operand must have an integer value (else runtime error). Emit `cmpf(Une, x, floor(x))` check before the bitwise op. ADR 0202 records.
- **NaN equality**: `NaN == NaN` is `false` in Lua (and in IEEE 754). Mixed-subtype `NaN == 1` is also false. ADR 0203 records.

## Scope (literal)

- ✅ This ADR is the **design entry** for the Integer/Float arc. Pins strategy, sub-ADR decomposition, and migration plan.
- ✅ Sub-ADRs 0197-0206 cover implementation. Each scopes its own surface per the table above.
- ✅ Migration in three phases (A additive, B opt-in, C strict) documented.
- ✅ Open questions enumerated and routed to specific sub-ADRs.
- ❌ Implement any of the above in this ADR. Decision-only.
- ❌ Commit to exact `ValueKind` naming (`Number` vs `Float`). Deferred to ADR 0198.
- ❌ Commit to exact `TAG_INTEGER` value. Deferred to ADR 0200.
- ❌ Cover 32-bit integer / single-precision float ports per Lua §2.1's mention. LuMeLIR's primary target is x86_64; 32-bit variants are Phase 5+ embedded work.

## Alternatives considered

- **Skip the subtype split and treat all numbers as f64 forever.** Rejected — disqualifies "Lua 5.4 conformant" claim; bitwise ops on float-only would surface UB or trap on every non-integer operand; `math.type` cannot be implemented honestly.
- **Promote everything to integer (use i64 throughout, lose float).** Rejected — breaks every fractional arithmetic test, surfaces precision loss, conflicts with `/` always-float requirement.
- **Use a single 64-bit tagged value (boxed in 16-byte slot) with NaN-tagging.** Rejected — adds complexity to every codegen site for marginal layout savings; the 16-byte tagged slot is already paid (ADR 0063 onwards). Subtype tag is the natural extension.
- **Bundle subtype split with `math` library expansion.** Rejected — `math` expansion needs the subtype landing first; bundling inflates ADR scope. Sequencing: subtype first, math after.
- **Defer to after Phase 4 closes.** Rejected per the roadmap recommendation: retrofit pressure on bitwise / math / metamethod dispatchers grows the longer it waits. Earlier is cheaper.

## Consequences

**Positive**
- Lua 5.4 spec conformance unlocked for `math.type`, `tointeger`, floor-div integer result, bitwise integer requirement, cross-subtype equality.
- Future stdlib expansion (`math.maxinteger`, `math.mininteger`, integer overflow semantics) becomes trivially possible.
- Bridge MVP (ADR 0191) gains a path to expose `extern "C" fn(i64) -> i64` Rust functions in addition to `f64`.

**Negative**
- Massive surface change touching parser, HIR, codegen, runtime, tagged ABI, every numeric stdlib.
- `print` format break for fractional-arithmetic-on-integer-valued-results case (`2.5 + 3.5` → `"6.0"` instead of `"6"`). Tests get updated.
- Phase B silent-demotion bridge state lasts ~5-8 sessions; partial conformance reads weird until Phase C lands.

**Locked in until superseded**
- "Integer subtype = i64, Float subtype = f64" is the contract. 32-bit variants are out of scope.
- Sub-ADR decomposition (0197-0206) is the implementation plan. Reordering within is acceptable; expanding beyond requires updating this ADR.
- Phase A / B / C migration is the contract; sub-ADRs cite their phase explicitly.

## Documentation updates

- [x] §8 — adds 0196.
- [x] Lua 5.4 conformance roadmap — "Integer/Float subtype" line cross-references this design entry.
- [x] ADR 0193 §Workstreams — Integer/Float subtype now has a design ADR.

## Test count delta

```
Step 0: 1431 (after 7473ae6)
C1 (this doc): 1431 → 1431 (decision-only, no test)
```

## Critical files

- `docs/design/0196-integer-float-subtype-design.md` (this doc).
- `docs/design/README.md` index entry.

Sub-ADR implementation will touch:

- `src/lexer/mod.rs` (integer vs float literal detection) — ADR 0197
- `src/parser/mod.rs` (literal AST) — ADR 0197
- `src/hir/mod.rs` (`ValueKind::Integer` + `infer_kind` arms) — ADR 0198
- `src/codegen/emit.rs` (i64 arithmetic / comparison paths) — ADRs 0199-0206
- `src/codegen/tagged.rs` (`TAG_INTEGER` + payload interpretation) — ADR 0200

## Risks

| Risk | Mitigation |
|---|---|
| Phase B silent demotion masks bugs | Each sub-ADR's C2 Red-Day-0 tests pin the integer path; demotion is opt-in per call site, not silent module-wide |
| Existing 1431 tests break en masse during Phase C | ADR 0204 pre-audits the affected set; migration is targeted, not whole-corpus rewriting |
| Tagged-ABI change (TAG_INTEGER) breaks every existing TaggedValue consumer | ADR 0200 introduces alias; consumer migration is per-ADR with audit pin |
| Bitwise ops on float surface unexpected trap | ADR 0202 documents the trap message + e2e test |
| Bridge MVP (ADR 0191) `rust_add(f64, f64) -> f64` becomes outdated when integer Bridge fns emerge | Separate ADR 0192's marshaling future-work; Bridge interaction is forward-compatible (existing f64 signatures stay valid) |

## Future work

- ADR 0197 — Lexer/parser integer vs float literal distinction
- ADR 0198 — HIR `ValueKind::Integer` + arithmetic-op result-kind rules
- ADRs 0199-0206 — codegen + tagged ABI + cross-subtype semantics per §Sub-ADR decomposition
- After 0206 — `math.type` / `math.tointeger` / `math.maxinteger` / `math.mininteger` stdlib lands; closes the integer-related Lua 5.4 §6.7 gaps

## References

- [Lua 5.4 Reference Manual §2.1 Values and Types](https://www.lua.org/manual/5.4/manual.html#2.1) — number subtype definition
- [Lua 5.4 Reference Manual §3.4.1 Arithmetic Operators](https://www.lua.org/manual/5.4/manual.html#3.4.1) — cross-subtype rules
- [Lua 5.4 Reference Manual §3.4.2 Bitwise Operators](https://www.lua.org/manual/5.4/manual.html#3.4.2) — integer requirement
- [Lua 5.4 Reference Manual §3.4.4 Concatenation](https://www.lua.org/manual/5.4/manual.html#3.4.4) — tostring(integer) vs tostring(float)
- [ADR 0063](0063-phase2-6c-tag-locals.md) — TaggedValue 16-byte layout (extended here)
- [ADR 0193](0193-phase4-entry-criteria.md) — Phase 4 entry; Integer/Float subtype is a Phase 4a workstream
- [Lua 5.4 Conformance Roadmap](../notes/lua54-conformance-roadmap.md) — top-priority recommendation
