# 0077. Phase 2.7p-arith-string-coerce: String → Number Arithmetic Coercion

- **Status:** Accepted (extended by ADR 0089 to runtime TaggedValue operands)
- **Date:** 2026-05-04
- **Deciders:** ShortArrow

> **ADR 0089 extension note (2026-05-10):** ADR 0077 covers the
> **static String** operand path (`HirExprKind::ArithStringCoerce`
> wrapping at HIR via `coerce_arith_operand_if_string`). ADR 0089
> extends the same Lua-spec §3.4.3 contract to **runtime String**
> operands held in `Local(TaggedValue)` slots, via the chokepoint
> `emit_load_tagged_operand_as_number` (consults
> `tagged.rs::policy_for_tagged_arith_operand`). The
> `emit_tonumber_for_arith` helper introduced here is reused
> (drop-in) by ADR 0089's chokepoint for the
> `CoerceStringToNumber` plan branch. The
> `s_arith_coerce_failed` global is reused for parse failures on
> both static and runtime paths; ADR 0089 adds a new
> `s_arith_on_non_numeric` global for the non-coercible
> tags (Bool/Nil/Function/Table/Deleted) which ADR 0077 did not
> cover (HIR rejects those at type-check time for static-kind
> operands).

## Context

Lua spec §3.4.1: arithmetic operators auto-coerce a string
operand when it parses as a number; otherwise the operation is
a runtime error. LuMeLIR previously rejected the entire `String
+ Number` shape at HIR time via the
`is_number_compatible(lk) && is_number_compatible(rk)` check
(`src/hir/mod.rs:2020`), so common Lua idioms like
`tonumber-input from io` or `"5" + 1` produced a static
`TypeMismatch` rather than the spec-correct runtime behaviour.

ADR 0028 (Phase 2.7e) shipped the `tonumber` builtin with a
NaN-sentinel-on-failure design (`sscanf("%lf")` returns NaN when
the parse fails). The arithmetic coercion path needs a similar
parser but with **different failure semantics**: silent NaN
propagation is wrong for arithmetic — the Lua spec requires a
runtime error.

Codex post-ADR-0076 review made this the **#1 priority**:

> 小さく、Lua 互換性の見返りが大きく、TaggedValue の未解決
> 大物にも依存しません。

The recommended approach (Plan A hybrid):
- HIR rewrites `String op Number` → `ArithStringCoerce(String) op
  Number`, satisfying the existing `is_number_compatible` check.
- Codegen lowers `ArithStringCoerce` via a new
  `emit_tonumber_for_arith` helper that **reuses
  `emit_tonumber`'s sscanf path** then **promotes the NaN
  sentinel into a runtime trap**.

## Decision

### HIR: `ArithStringCoerce` wrapper variant

```rust
/// Phase 2.7p-arith-string-coerce (ADR 0077): wraps a String
/// operand of an arithmetic / bitwise BinOp with runtime
/// numeric coercion. Codegen lowers via
/// `emit_tonumber_for_arith` which traps on parse failure.
ArithStringCoerce(Box<HirExpr>),
```

`infer_kind(ArithStringCoerce(_)) = ValueKind::Number`. The
wrapper is opaque to all other HIR analyses — `Builtin::ToNumber`
and `is_number_compatible` retain their existing semantics.

### HIR: `coerce_arith_operand_if_string` helper (Codex Tidy First)

```rust
fn coerce_arith_operand_if_string(
    expr: HirExpr,
    locals: &[LocalInfo],
    functions: &[HirFunction],
) -> HirExpr {
    if matches!(infer_kind(&expr, locals, functions), ValueKind::String) {
        let span = expr.span;
        HirExpr {
            kind: HirExprKind::ArithStringCoerce(Box::new(expr)),
            span,
        }
    } else {
        expr
    }
}
```

Applied to both operands at the top of the BinOp arithmetic /
bitwise arm in `lower_expr`, **before** the kind validation.
String operands wrap; everything else passes through; the
existing `is_number_compatible` check then sees the wrapper as
a Number.

### Codegen: `emit_tonumber_for_arith` helper (Codex Tidy First)

```rust
fn emit_tonumber_for_arith<'a, 'c>(
    context: &'c Context,
    block: &'a Block<'c>,
    str_ptr: Value<'c, 'a>,
    types: &Types<'c>,
    loc: Location<'c>,
) -> Value<'c, 'a> {
    let parsed = emit_tonumber(context, block, str_ptr, ValueKind::String, types, loc);
    // arith.cmpf(Une, x, x) is true exactly when x is NaN.
    let is_nan = block
        .append_operation(arith::cmpf(context, CmpfPredicate::Une, parsed, parsed, loc))
        .result(0).unwrap().into();
    // scf.if is_nan → exit_with_message(s_arith_coerce_failed)
    ...
    parsed
}
```

The helper is purely additive — `emit_tonumber` itself is
unchanged, so the `Builtin::ToNumber` arm (`tonumber("abc") →
NaN sentinel` per ADR 0028) remains intact. The arithmetic path
gets its own failure semantics via the wrapping `scf.if` trap.

New string global `s_arith_coerce_failed` carries the
diagnostic message (`"attempt to perform arithmetic on a string
value"`) — matches the standard Lua interpreter's wording.

### `emit_expr` arm

```rust
HirExprKind::ArithStringCoerce(operand) => {
    let str_ptr = emit_expr(context, block, operand, ...)?;
    Ok(emit_tonumber_for_arith(context, block, str_ptr, types, loc))
}
```

`collect_string_pool`'s walker also gets an arm so the wrapped
operand's string literal lands in the pool.

### Hex floats — work without extra effort

glibc's `sscanf("%lf")` accepts hex-prefixed floats (`"0x10"`
parses to 16). Lua spec also expects this. Pinned by
`string_arith_hex_works_via_glibc_sscanf` so a libc change
(e.g. moving to musl, or running a future host where
`sscanf` rejects hex) surfaces immediately. No LIC needed.

## Alternatives Considered

- **Plan B (codegen runtime kind dispatch)**: HIR keeps the
  current rejection; codegen detects mixed-kind operands at
  runtime and inserts coercion. Rejected — would require
  threading kind information through the runtime layer and
  expanding the static-kind contract every codegen site relies
  on. Heavy for the gain.
- **Plan C (lazy evaluation)**: defer operand evaluation until
  runtime kind dispatch decides whether to coerce. Rejected —
  destroys the HIR/codegen separation that ADR 0073's split
  established.
- **Reuse `Builtin::ToNumber` directly**: have HIR rewrite
  `String op Number` → `Call(Builtin::ToNumber, [String]) op
  Number`. Tempting (no new HIR variant) but conflates two
  failure semantics: the builtin path returns NaN, the arith
  path traps. Mixing them under one codegen call site means
  one of the two contracts must yield. Rejected to preserve
  ADR 0028.

## Consequences

- **`LIC-2.7p-arith-coerce-1` → resolved.** Lua spec §3.4.1
  arith coercion supported for static-String operands.
- **New `LIC-2.7p-arith-coerce-tagged-1`** (pending): a
  `Local(TaggedValue)` operand whose runtime tag may be
  `TAG_STRING` is not coerced — HIR can't prove the kind
  statically. The trapping number-only path applies (existing
  ADR 0063 behaviour). Unlocking this requires a runtime tag
  dispatch in arith codegen; deferred.
- **Test totals: 888 → 898 green.** 10 new e2e tests covering
  decimal / float / negative / scientific / hex / both-string
  positive cases, the unparseable trap, and 2 regression tests
  (pure-number arith, tonumber NaN sentinel).
- **HIR**: +30 LOC (`ArithStringCoerce` variant + `infer_kind`
  arm + `coerce_arith_operand_if_string` helper + lower_expr
  rewrite).
- **Codegen**: +60 LOC (`emit_tonumber_for_arith` helper +
  emit_expr arm + `collect_string_pool` arm + new
  `s_arith_coerce_failed` global).
- The 12 arithmetic / bitwise BinOps (`+ - * / // % ^ & | ~ <<
  >>`) all accept String operands transparently. Comparison
  operators (`< <= > >=`) and equality (`== ~=`) keep their
  existing semantics — Lua spec disallows / no-coerces those
  forms anyway.

## Documentation updates

- [x] §1 slot layout — n/a (no tag space change).
- [x] §2 producer / source taxonomy — n/a (the wrapper is a
      passthrough at HIR analysis time).
- [x] §3 consumer coverage matrix — Arith / ordering row
      annotated: "String operand → numeric parse via
      `emit_tonumber_for_arith`; runtime trap via
      `s_arith_coerce_failed` on parse failure (ADR 0077)".
- [x] §4 LIC consolidation — `arith-coerce-1` resolved,
      `arith-coerce-tagged-1` added pending. Totals: **18
      resolved / 1 partial / 2 pending**.
- [x] §5 runtime tag invariants — n/a.
- [x] §6 cross-reference — note the distinction between the
      `Builtin::ToNumber` NaN-sentinel contract (ADR 0028) and
      the `ArithStringCoerce` trap-on-failure contract (this
      ADR).
- [x] §7 open questions — `arith-coerce` removed; `iteration
      pairs/ipairs` promoted to #1.
- [x] §8 ADR index — ADR 0077 row added; "Last updated" stamp
      bumped.

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative
list (ADR 0068).
