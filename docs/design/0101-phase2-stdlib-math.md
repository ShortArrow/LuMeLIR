# 0101. Phase 2.7q-stdlib-math: math.* Builtins (sqrt / floor / abs)

- **Status:** Accepted
- **Date:** 2026-05-16
- **Deciders:** ShortArrow

## Replan provenance

ADR 0091-0100 (10 consecutive ADRs) closed the method-axis
refinement chain. Remaining carry-overs from that chain are small
and diminishing-returns. ADR 0101 pivots to a new axis: **stdlib
extensions**, starting with the math.* library.

```lua
print(math.sqrt(16))    -- → 4
print(math.floor(3.7))  -- → 3
print(math.abs(-5))     -- → 5
```

First stdlib addition since Phase 2.5+ when the existing builtins
(print / tostring / tonumber / type / assert / error / next) landed.
Establishes the pattern for further stdlib expansion (math.*
continuation, string.*, table.*, io.*).

Codex post-0100 review (6 視点) verdict: **Refactor → Go**. Critical:
- **builtin 化条件を「`math` が未解決識別子のときだけ」に厳密限定する**.
  User shadowing `local math = ...` MUST resolve through the
  normal table path (Lua-spec correct).
- TDD must include shadowing positive pin + unknown-method
  negative pin.

## Non-goals (top-of-ADR — codex critical)

- **Other math functions** — `math.pi`, `math.random`, `math.sin`,
  `math.cos`, `math.log`, `math.pow`, etc. Future ADRs add
  incrementally.
- **string.* / table.* / io.* libraries** — separate ADRs.
- **`math` as a Lua module / namespace** — `math` is recognized
  ONLY as the bare unresolved identifier in `Call{Index{Ident("math"),
  Str(method)}, args}` shape. No first-class chunk-scope `math` table.
- **User shadowing of `math` (Codex critical)** — `local math = {};
  math.foo = function(x) return x end; math.foo(4)` must respect
  the user's table. Dispatch falls through to the normal Index-callee
  Call path.
- **`math.unknown(x)`** — surfaces as UndefinedName (since `math` is
  unresolved) or similar HIR-time error. NOT silently builtin-ized.
- **`local m = math.sqrt; m(x)` rebind / alias** — ADR 0098-0100
  alias_map only resolves through `method_funcs`, which doesn't
  include math.*. Future ADR may extend.
- **HIR refinement for math.* args** — the math fns take Number;
  no String/Bool/Table refinement needed.

## Context

Pre-ADR-0101 `lower_call` dispatch:
```
match callee.kind:
    Ident(name) → Builtin::from_name / User / UnknownFunction
    Index{...} → ADR 0091 IndexCallee path → IndirectDispatch
    MethodCall{...} → ADR 0092 MethodCall desugar
```

`math.sqrt(x)` parses as `Call{Index{Ident("math"), Str("sqrt")}, args}`.
Lowering today fails: `math` is unresolved → HIR `lower_expr` of
target Ident emits UndefinedName before the Index ever resolves.

## Reframing

Add a strict-shape dispatch BEFORE the existing `classify_callee_form`:

```
match callee.kind:
    Index{target: Ident("math"), key: Str(method)}
            when resolve("math").is_none()
                 AND function_names.contains_key("math") == false
                 AND Builtin::math_from_method(method) = Some(b):
        → lower_math_builtin_call(b, args, span) → Call{Builtin(b)}
    (else)
        → existing dispatch
```

Strict guards:
- `target_name == "math"`: shape predicate.
- `resolve("math").is_none()`: user shadowing respected (Codex
  critical).
- `!function_names.contains_key("math")`: defensive against a
  top-level `local function math() end` shadow.
- `Builtin::math_from_method(method)` returns Some: only recognized
  methods are builtin-ized; unknown methods fall through.

`Builtin::math_from_method` maps `"sqrt"` / `"floor"` / `"abs"`
to the new variants. Other names return None.

Codegen path:
- `emit_libm_decls` extended with extern `sqrt(f64) -> f64` and
  `fabs(f64) -> f64` (floor was already declared via prior
  bitwise/arith path).
- New `emit_libc_call_f64` helper in `primitive.rs` mirrors the
  i32/i64/ptr/void variants.
- Builtin emit dispatch: `MathSqrt → "sqrt"`, `MathFloor → "floor"`,
  `MathAbs → "fabs"`. Direct libm call returning f64.

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First**: chokepoint dispatch at
  `lower_call` entry, single helper `lower_math_builtin_call`.
- [x] **#2 TDD (Codex critical)**: 6 tests — 3 happy + 1 regression
  + 1 shadowing positive pin + 1 unknown-method negative pin.
- [x] **#3 FP**: shape predicate is pure pattern match + name
  resolution lookup. No side effects.
- [x] **#4 CA**: HIR (ir.rs Builtin variants + mod.rs dispatch) +
  codegen (libm decl + emit arm + helper). Codegen delta bounded.
- [x] **#5 Security (Codex critical)**: `resolve("math").is_none()`
  + `!function_names.contains_key("math")` guards respect Lua
  shadowing semantics.
- [x] **#6 Documentation**: ADR 0101 with stdlib-axis-begin framing,
  shadowing critical, non-goals enumerated. Codex template AGENTS
  row.

## Test count delta

```
Step 0:  1060 → 1063 (3 Red + 3 already-green: regression-pin +
                       shadowing positive + unknown-method negative)
Step 1:  1060 → 1063 (Builtin variants; tests still Red)
Step 2:  1060 → 1063 (HIR dispatch; codegen not yet handles new
                       variants → still Red)
Step 3:  1060 → 1066 (codegen emit arms; 3 Red → Green)
Step 4:  1060 → 1066 (clippy + fmt)
Step 5:  1060 → 1066 (docs only)

Final: 1060 → 1066 green, single atomic commit
  feat(hir,codegen,docs): stdlib math.* builtins (sqrt/floor/abs) (ADR 0101)
```

## Verification

- `cargo test --no-fail-fast` → **1060 → 1066**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/cli/ src/pipeline.rs src/parser/ src/lexer/` → **0**
- `git diff --stat src/codegen/` → ~67 LOC (libm decl + emit arm +
  helper)
- Manual smoke:
  ```bash
  echo 'print(math.sqrt(16))
  print(math.floor(3.7))
  print(math.abs(-5))' > /tmp/m.lua
  cargo run --quiet -- compile /tmp/m.lua && /tmp/m
  # Expected: 4 / 3 / 5
  ```

## Future work

- **Other math.* functions** — `math.pi` constant (different shape:
  Index without Call), `math.random` (RNG state), `math.sin/cos/tan`,
  `math.log/exp`, `math.pow` (libm `pow` already declared, just
  needs Builtin variant + emit arm).
- **string.* library** — `string.len`, `string.sub`, `string.upper`,
  `string.lower`. Same shape pattern as math.*.
- **table.* library** — `table.insert`, `table.remove`,
  `table.concat`. More complex (table operations vs scalar libm).
- **io.* library** — `io.read`, `io.write`. Even more complex.
- **`local m = math; m.sqrt(x)` aliasing** — would require
  introducing `math` as a first-class chunk-scope binding. Out of
  MVP.
- **Method-axis carry-overs** — control-flow refinement,
  function-body alias, source-order shadowing.

## ADR number / phase tag

ADR 0101 = Stdlib math.* Builtins. Phase tag: `2.7q-stdlib-math`
(new sub-lane). Pivots from the ADR 0091-0100 method-axis
refinement chain to the stdlib axis.
