# 0141. Anonymous Function Param-Kind Refinement from `IndexAssign` Sites

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-31
- **Deciders:** ShortArrow

## Context

Tier 1 metatables sweep complete (ADRs 0137 – 0140). The next gating issue for Tier 2 metamethod ADRs (`__tostring`, `__concat`, comparison) is **not** the call ABI, as initially diagnosed — it is **param-kind inference for anonymous functions stored in table slots**.

The natural Lua idiom for metamethods is:

```lua
local mt = {}
mt.__tostring = function(t) return "Vec" end
print(mt.__tostring(some_table))
```

Today the anonymous `function(t) return "Vec" end` registers a fresh `HirFunction` with **default `params[0].kind = Number`** (`src/hir/mod.rs:3923`). When `mt.__tostring(some_table)` lowers via ADR 0091's synth-local materialisation → ADR 0082's `Callee::IndirectDispatch`, the candidate filter `compatible_user_functions` (`src/hir/mod.rs:4165`) looks for a user fn with `param_kinds = [Table]` and finds none. Result:

```
hir error: indirect call through TaggedValue local '__callee_0' has no
compatible user function in this module (param_kinds=[Table], …; ADR 0082)
```

ADRs 0093 / 0094 / 0096 / 0097 / 0098 / 0100 already added refinement for `function obj.method(...)` (MethodDef) syntax and `local g = obj.method` rebinds via the `method_funcs` / `alias_map` pre-registration mechanism. The missing case is **`mt.k = function(...) end`** — anonymous FunctionExpr value in a top-level `IndexAssign`.

## Scope (literal)

**Pass-1 walk over `IndexAssign(target, Str(key), FunctionExpr(...))` at chunk top-level**, where `target` extracts via `extract_index_chain` (ADR 0097 helper). For each such site:

1. Pre-allocate a `FuncId` for the anonymous FunctionExpr.
2. Register `(chain, key) → fid` in `method_funcs`.
3. Track pre-allocated FuncIds in `anon_indexassign_func_ids: Vec<FuncId>` (source-order Vec, mirrors `methoddef_func_ids` from ADR 0093).

The existing Pass-1.5 walker (`infer_user_function_param_kinds`, `src/hir/mod.rs:662`) already refines via `method_funcs.get(&(chain, method)).map(|fid| try_refine_func_args(fid, 0, args, ...))` (line 877-879). Pre-registration makes anonymous FunctionExpr sites visible to this path **without any walker changes**.

Pass-2 FunctionExpr lowering consumes the pre-allocated FuncId via a counter (`anon_indexassign_seq`, mirrors `funcdef_seq` from ADR 0042) and seeds `external_kinds` from `self.functions[id.0].params[j].kind` (Pass-1.5 refined) instead of the hardcoded `vec![Number; n]`.

Out of scope:

- ❌ Nested `IndexAssign(t.k = function...)` **inside function bodies**. Top-level only.
- ❌ Non-Str keys (`t[expr] = function...`). Static String key only.
- ❌ FunctionExpr inside more complex shapes (`t.k = (cond and fn1 or fn2)`). Direct assignment only.
- ❌ Non-anonymous FunctionRef values (`mt.k = some_named_fn`). Already handled via `alias_map`.
- ❌ Param-kind refinement for capturing closures with upvalues. Default to `Number` placeholder; closures rarely need cross-table dispatch.

## Decision

### Pass-1 registration (new walk, `src/hir/mod.rs::lower`)

After the existing MethodDef walk (line 1291-1315) and before the alias_map walk (line 1333), add:

```rust
let mut anon_indexassign_func_ids: Vec<FuncId> = Vec::new();
for stmt in chunk {
    if let StmtKind::Assign { targets, value, .. } = &stmt.kind
        && let Some(target) = targets.first()
        && let Some((chain, key)) = extract_index_chain(target)
        && let ExprKind::FunctionExpr { params, .. } = &value.kind
    {
        let fid = alloc_anon_signature(params, &mut functions, ParentScope::Chunk);
        anon_indexassign_func_ids.push(fid);
        // Pre-register so infer_user_function_param_kinds picks it
        // up via the existing (chain, key) → FuncId lookup.
        method_funcs.entry((chain, key)).or_insert(fid);
    }
}
```

`extract_index_chain` already returns `Option<(Vec<String>, String)>` for `Index { target: chain_of_idents, key: Str(s) }` patterns.

`alloc_anon_signature` is a new helper (mirrors `alloc_method_signature` from ADR 0093) that pushes a placeholder `HirFunction` with `params` filled by name and `kind = Number` default.

**`or_insert` (not `insert`)** — MethodDef registration takes precedence on conflict. Anonymous IndexAssign at top-level of a chunk that ALSO has `function chain.key(...) end` for the same chain/key falls back to MethodDef semantics.

### Pass-2 FunctionExpr lowering

In `LowerCtx`, add `anon_indexassign_seq: usize` and `anon_indexassign_func_ids: Vec<FuncId>` (snapshot from chunk-level Pass-1).

The Pass-2 walker, when processing a top-level `StmtKind::Assign { target=index_chain, value=FunctionExpr(...) }`, recognises the same shape and **consumes the next FuncId from `anon_indexassign_func_ids[anon_indexassign_seq++]`** rather than allocating fresh.

The FunctionExpr lowering arm at `src/hir/mod.rs:3908` is modified to optionally accept a pre-allocated FuncId. When provided, it uses that ID instead of `FuncId(self.functions.len())` and seeds `external_kinds` from `self.functions[id.0].params.iter().map(|p| p.kind).collect()` (Pass-1.5 refined kinds) instead of the hardcoded `vec![Number; n]`.

### What this unlocks

`mt.__tostring = function(t) return "Vec" end` followed by `mt.__tostring(some_table_local)` now:

1. Pass-1 pre-registers `( ["mt"], "__tostring" ) → fid_anon`.
2. Pass-1.5 walker sees `Call(Index(Ident("mt"), Str("__tostring")), [some_table_local])`, looks up `method_funcs`, refines `functions[fid_anon].params[0].kind = Table`.
3. Pass-2 lowers the FunctionExpr with `external_kinds = [Table]`, so the body's `t` Local has kind `Table` — ret_kinds inference picks up `[String]`.
4. The call site lowers via ADR 0091 → 0082, `compatible_user_functions(sig={param=[Table], ret=[String]}, ...)` finds `fid_anon` — dispatches.

## Alternatives considered

- **New `Callee::CallThroughTableMember` variant** (original Path β plan). Rejected after exploration showed ADR 0091 → 0082 already provides the dispatch chain; the gap is purely param-kind refinement, not call ABI.
- **Two-pass HIR (lower once with defaults, collect evidence, re-lower)**. Rejected — quadratic cost, redundant work, contrary to ADR 0091's pre-registration pattern.
- **Defer this gap and require users to write `local function fn(t) ... end; mt.k = fn`**. Rejected — the anonymous idiom is canonical Lua for metamethods; requiring named functions creates a deviation that surprises every user.
- **Refine via the synth-local introduced by ADR 0091** (`local __callee_0 = mt.k; __callee_0(arg)` — the `__callee_0`'s declared kind is TaggedValue, but the underlying FunctionExpr's params could be refined by looking through the IndexAssign provenance). Rejected — the synth local is created at Pass-2 time, after Pass-1.5 has already locked param kinds; pre-registration at Pass-1 is structurally simpler.

## Consequences

**Positive**
- The natural Lua metamethod assignment idiom works. Tier 2 ADRs 0142 (`__tostring`), 0143 (`__concat`), 0144 (comparison metamethods) become straightforward consumer additions on top of this ABI.
- The pre-registration mechanism is consistent across `FunctionDef` (ADR 0042), `MethodDef` (ADR 0093), `local g = obj.method` (ADR 0098), and now anonymous-FunctionExpr-in-IndexAssign — one architectural pattern, four producers.
- Zero new HIR `Callee` variant. Zero new codegen path. The call ABI work is purely HIR-side.

**Negative**
- Pass-1 walk now visits one more chunk-level shape. O(N) extra per-chunk cost, negligible.
- `LowerCtx` carries one more counter (`anon_indexassign_seq`) and one more vec (`anon_indexassign_func_ids`). One extra field each.
- The "or_insert" precedence rule means a chunk with BOTH `mt.k = function...` and `function mt.k(...)` will silently prefer the MethodDef path. Documented as the expected resolution; no observable surprise because Lua source-level semantics of having both is ambiguous anyway.

**Locked in until superseded**
- Top-level scope. Nested IndexAssign in function bodies remains unrefined. Future ADR may extend.
- Static String key only. Dynamic-key refinement requires constant-prop, deferred.

## Documentation updates

- [x] §1–§5 — **no change** (HIR-only refactor, no slot-layout / producer / consumer impact).
- [x] §4 LIC consolidation — new resolved entry `LIC-anon-fn-indexassign-refine-1`.
- [x] §7 open questions — closes "anon FunctionExpr param refinement" open item; opens "nested-IndexAssign FunctionExpr refinement" as new future work.
- [x] §8 ADR index — adds 0141.

## Test count delta

```
Step 0:   1310 (a051be8)
C2 (5 new e2e Red Day 0):  1310 → 1310 (existing green)
C3 (Pass-1 walk + Pass-2 FuncId reuse): 1310 → 1315 (all green)
```

## Critical files

- `src/hir/mod.rs`:
  - `lower()` — new Pass-1 walk for IndexAssign+FunctionExpr (~30 LOC).
  - New helper `alloc_anon_signature` (~15 LOC, mirrors `alloc_method_signature`).
  - `LowerCtx::new` — accept `anon_indexassign_func_ids` (+1 field, +1 init).
  - Pass-2 main walker — consume FuncId from counter for matching IndexAssign shapes (~25 LOC).
  - `lower_expr` `FunctionExpr` arm — accept optional pre-allocated FuncId (~20 LOC delta).
- `tests/phase2_6plus_anon_fn_indexassign_refine.rs` (NEW) — 5 e2e.
- `docs/design/tagged-semantics.md` — §4 / §7 / §8.

## Risks

| Risk | Mitigation |
|---|---|
| Pre-registration of an anonymous FuncId in `method_funcs` shadows a same-key MethodDef | `or_insert`: existing MethodDef entries win on conflict (precedence rule documented). Test 5 pins. |
| Pass-2 counter desync if walker visits IndexAssign shape differently than Pass-1 | Both walks iterate `chunk` linearly with the same `if let` filter. Counter is initialized to 0 and incremented in lockstep; mismatch surfaces as a panic in `anon_indexassign_func_ids[i]` access, caught immediately by Red Day 0 tests. |
| FunctionExpr nested inside non-IndexAssign expressions still defaults to Number | Out of scope; documented. Test 4 negative-pin confirms behaviour is unchanged for non-IndexAssign anonymous functions. |
| `extract_index_chain` rejects valid shapes (e.g. `(a or b).k = function...`) | The non-Ident-chain prefix returns None — falls through to the existing "fresh FuncId" path. Behaviour is unchanged, not regressed. |
| ADR 0098 `alias_map` interaction (`local g = mt.fn` after `mt.fn = function...`) | `alias_map` is populated in Pass-1 ROUND 2 (insert-only) AFTER the new pre-registration walk. The alias resolves to the pre-allocated FuncId via the same `method_funcs` entry. Test 3 pins. |

## Future work

- **ADR 0142 = `__tostring` metamethod consumer** — `tostring(t)` builtin probes `mt.__tostring` and dispatches via the now-refined anon FunctionExpr.
- Nested-IndexAssign FunctionExpr refinement (function bodies).
- Dynamic-key refinement via constant-prop.
- Capturing closure with upvalues — param refinement when the closure escapes via IndexAssign.

## References

- [ADR 0042](0042-phase2-5c1-top-level-capture.md) — Pass-2 source-order walk pattern (FuncId pre-allocation via Pass-1).
- [ADR 0082](0082-phase2-5x-callee-dispatch.md) — `Callee::IndirectDispatch` + `compatible_user_functions`.
- [ADR 0091](0091-phase2-callee-normalization.md) — Index-Callee Call synth-local materialisation.
- [ADR 0093](0093-phase2-method-arg-refine.md) — `method_funcs` Pass-1 registration + Pass-1.5 refinement (closest precedent).
- [ADR 0094](0094-phase2-method-idx-call-refine.md) — Index-callee Call refinement walker (existing `Call(Index(chain, key), args)` arm reused here).
- [ADR 0096](0096-phase2-multi-segment-method-def.md) — Multi-segment method def, chain-keyed `method_funcs`.
- [ADR 0097](0097-phase2-multi-seg-call-refine.md) — `extract_index_chain` helper (reused for Pass-1 walk).
- [ADR 0098](0098-phase2-name-rebind-refine.md) — `alias_map` interaction with `method_funcs`.
- [ADR 0133](0133-phase2-completion-criteria.md) — Phase 2 deferral table; this ADR unblocks the metamethod rows.
