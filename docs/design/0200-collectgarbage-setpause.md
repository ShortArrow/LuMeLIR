# 0200. `collectgarbage("setpause", n)` — Configurable Threshold Multiplier

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-16
- **Deciders:** ShortArrow

## Context

[ADR 0186](0186-gc-auto-trigger-and-func-factoring.md) §Future work named `collectgarbage("setpause", n)` as deferred. [Bucket D probe results](../notes/bucket-d-probe-results.md) §0186 confirmed the gap: `collectgarbage("setpause", 200)` fails with `ArityMismatch { builtin: "collectgarbage", expected: 0, actual: 2 }`.

Lua 5.4 §6.1: `collectgarbage("setpause", n)` sets the GC pause multiplier (as a percentage). Default = 200 (heap doubles between collections). The current ADR 0186 doubling logic uses a hardcoded `*2`; this ADR makes it configurable via a new `g_gc_pause` mutable global.

## Scope (literal)

- ✅ Relax `Builtin::CollectGarbage` arity from `(0, 1)` to `(0, 2)`.
- ✅ Accept `collectgarbage("setpause", Number)` at HIR (extend the existing literal-Str gate).
- ✅ New `g_gc_pause` mutable i64 global, init `GC_PAUSE_INIT = 200`.
- ✅ `emit_gc_alloc` doubling logic uses `g_gc_pause / 100` instead of hardcoded `* 2`. Result preserves current behaviour (200/100 = 2.0× same as before).
- ✅ `Callee::Builtin(CollectGarbage)` emit arm — 2-arg "setpause" form loads previous `g_gc_pause`, stores new value, returns previous as f64.
- ❌ Other options: `"stop"` / `"restart"` / `"step"` / `"setstepmul"`. Each gets a separate follow-up ADR if/when needed.
- ❌ Generational GC params (`"generational"` / `"incremental"`). Lua 5.4 deprecates `setpause`/`setstepmul` in favour of these; out of scope for this MVP.
- ❌ Argument validation beyond "Number". Negative values, zero, or values < 100 are accepted at face value (Lua spec doesn't strictly bound; user is responsible).

## Decision

### `src/hir/ir.rs`

```rust
Builtin::CollectGarbage => (0, 2),  // was (0, 1)
```

### `src/hir/mod.rs::lower_builtin_call`

Extend the existing `Str("count")` literal gate at line ~5344 to also accept `"setpause"` for the 1-arg form (signature change to fit 2-arg). For the 2-arg form, validate arg 0 is `Str("setpause")` and arg 1 is Number-kind:

```rust
if matches!(builtin, Builtin::CollectGarbage) && lowered_args.len() == 1 {
    match &lowered_args[0].kind {
        HirExprKind::Str(s) if s == "count" => {}
        ... // existing reject paths
    }
}
if matches!(builtin, Builtin::CollectGarbage) && lowered_args.len() == 2 {
    let opt_ok = matches!(&lowered_args[0].kind, HirExprKind::Str(s) if s == "setpause");
    if !opt_ok { /* TypeMismatch */ }
    let val_kind = infer_kind(&lowered_args[1], &self.locals, &self.functions);
    if !matches!(val_kind, ValueKind::Number) { /* TypeMismatch */ }
}
```

### `src/codegen/tagged.rs`

```rust
pub(crate) const GC_PAUSE_INIT: i64 = 200; // Lua 5.4 default
```

### `src/codegen/emit.rs::emit_module_init`

Register `g_gc_pause` next to `g_gc_threshold`:

```rust
emit_mutable_i64_global(context, module, types, "g_gc_pause", GC_PAUSE_INIT, loc);
```

### `src/codegen/primitive.rs::emit_gc_alloc` doubling

Replace the `arith::muli(post_sweep, two_const)` step with `(post_sweep * g_gc_pause) / 100`. Constant `200` produces 2.0× → identical observable behaviour for the v1 safety-mode test corpus.

### `Callee::Builtin(CollectGarbage)` 2-arg "setpause" arm

```rust
if args.len() == 2 {
    // Lua spec: "setpause" returns the previous value as integer
    // (we use f64 throughout). Store the new value, return previous.
    let pause_addr = emit_addressof(context, block, "g_gc_pause", types, loc);
    let prev = emit_load(block, pause_addr, types.i64, loc);
    let new_f64 = emit_expr(context, block, &args[1], ..., loc)?;
    let new_i64 = emit_f2i(block, new_f64, types, loc);
    emit_store(block, new_i64, pause_addr, loc);
    let prev_f64 = arith::sitofp(prev, types.f64, loc);
    return Ok(prev_f64);
}
```

### Test

`tests/phase4_collectgarbage_setpause.rs` (NEW, 1 e2e):

```lua
local prev = collectgarbage("setpause", 150)
print(prev)  -- 200 (default)
```

## Alternatives considered

- **Bundle all of stop/restart/step/setpause/setstepmul.** Rejected — each touches different runtime state; bundling inflates ADR. `setpause` is the only one with concrete current-state coupling (ADR 0186 doubling).
- **Use Number directly in `g_gc_pause` instead of i64.** Rejected — pause is integer percentage by spec; i64 matches the existing threshold globals.
- **Skip the global and inline a constant.** Rejected — defeats the purpose; setpause must be observable across calls.

## Consequences

**Positive**
- Lua 5.4 `setpause` API works for the documented use case (single setter call).
- ADR 0186 doubling becomes configurable without runtime restart.
- Bucket D D-§0186-setpause RESOLVED.

**Negative**
- One more mutable module global. Trivial.

**Locked in until superseded**
- `g_gc_pause = 200` default matches Lua spec.
- 2-arg arity. Future `stop`/`restart`/`step` arms extend the same emit arm; no further arity change needed.

## Documentation updates

- [x] §8 — adds 0200.
- [x] Bucket D §0186-setpause marked RESOLVED.

## Test count delta

```
Step 0: 1442 (after fd3b18b)
C1 (doc): 1442 → 1442
C2 (1 Red Day 0 e2e): 1442 → 1442 (Red — ArityMismatch)
C3 (impl): 1442 → 1443 (Green)
```

## Critical files

- `docs/design/0200-collectgarbage-setpause.md` (this doc).
- `docs/design/README.md` index entry.
- `src/hir/ir.rs::arity` — CollectGarbage `(0,1)` → `(0,2)`.
- `src/hir/mod.rs::lower_builtin_call` — accept `setpause`+Number.
- `src/codegen/tagged.rs` — `GC_PAUSE_INIT` constant.
- `src/codegen/emit.rs::emit_module_init` — `g_gc_pause` global registration.
- `src/codegen/emit.rs::CollectGarbage emit arm` — 2-arg setpause path.
- `src/codegen/primitive.rs::emit_gc_alloc` — doubling uses `g_gc_pause / 100`.
- `tests/phase4_collectgarbage_setpause.rs` (NEW) — 1 e2e.

## Risks

| Risk | Mitigation |
|---|---|
| Existing v1 safety-mode regression net breaks if doubling changes from `*2` to `*g_gc_pause/100` | Default `g_gc_pause = 200`; `200/100 = 2` so observable behaviour unchanged. |
| f64-to-i64 conversion of pause value loses precision for very large pauses | i64 range is 64-bit signed; pause values are in [1, 1000] range realistically. |
| Future `stop`/`restart`/`step` ADRs reshape the dispatch | Acceptable — same arm extension pattern. |

## Future work

- ADR (TBD) — `collectgarbage("stop" / "restart" / "step" / "setstepmul")`.
- ADR (TBD) — Generational GC params (`"generational"` / `"incremental"`).

## References

- [Lua 5.4 §6.1 collectgarbage](https://www.lua.org/manual/5.4/manual.html#pdf-collectgarbage)
- [ADR 0186](0186-gc-auto-trigger-and-func-factoring.md) — auto-trigger threshold + doubling.
- [Bucket D probe results §0186](../notes/bucket-d-probe-results.md)
