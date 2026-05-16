# 0100. Phase 2.6+-reassign-alias: Re-Assignment Alias via StmtKind::Assign Extension + Helper Extract

- **Status:** Accepted
- **Date:** 2026-05-16
- **Deciders:** ShortArrow

## Replan provenance

ADR 0098 / 0099 alias_map walker handled only `StmtKind::Local` /
`LocalMulti`. Re-assignment via `StmtKind::Assign` / `AssignMulti`
was the remaining gap:

```lua
local g = app.dummy        -- alias_map["g"] = dummy (ADR 0098)
g = app.format             -- Assign — today NOT seen by alias_map
g("world")                 -- IndirectCallNoCandidates / wrong dispatch
```

Codex post-0099 review (6 視点) verdict: **Refactor → Go**. Critical:
- **Extract `record_alias_binding` helper** so Local / Assign ×
  LocalMulti / AssignMulti × Round 1 / Round 2 don't duplicate
  (8 arms → 1 helper + 1 dispatch).
- **TDD**: add conditional-flow non-refinement negative pin
  (boundary documentation via test).
- **Documentation**: strong §Non-goals — top-level-only,
  control-flow-non-supported, call-before-assign-unresolved.

## Non-goals (top-of-ADR — codex critical)

- **Function-body re-assignment** — Pass-1 walk is TOP-LEVEL only.
- **Control-flow aware refinement** — `if cond then g = a.b.m
  end; g()`. The walker does NOT descend into `if`/`while`/`for`
  bodies; conditional Assigns inside don't update alias_map.
  The OUTER Local init's alias governs refinement.
- **Call-before-assign source-order resolution** — `local g =
  a.b.m1; g(x); g = a.b.m2; g(y)`. Today's last-wins applies
  chunk-wide; `g(x)` may refine via m2.
- **Method-call rebind via Assign** — `g = a:m` (MethodCall AST,
  not Index chain).
- **Codegen changes** — `emit.rs` zero-diff (HIR-only).

## Context

Pre-ADR-0100 alias_map build had 4 arms duplicated:

```
Round 1 (last-wins, Index-chain):
  Local arm
  LocalMulti arm
Round 2+ (insert-only, Ident-rebind):
  Local arm (with insert-only guard)
  LocalMulti arm (with insert-only guard)
```

Adding Assign + AssignMulti to both rounds would double this to 8
arms. Codex critical: factor into `record_alias_binding` helper.

## Reframing

Helper unifies all combinations:

```rust
fn record_alias_binding(
    name, value, alias_map, method_funcs, insert_only,
) -> bool {
    if insert_only && alias_map.contains_key(name) { return false; }
    // Index-chain path
    if let Some((chain, method)) = extract_index_chain(value)
        && let Some(&fid) = method_funcs.get(&(chain, method)) {
        alias_map.insert(name, fid); return true;
    }
    // Ident-rebind path
    if let ExprKind::Ident(other) = &value.kind
        && let Some(&fid) = alias_map.get(other) {
        alias_map.insert(name, fid); return true;
    }
    false
}

fn process_alias_stmt(stmt, alias_map, method_funcs, insert_only) -> bool {
    match &stmt.kind {
        Local | Assign { name, value } => record_alias_binding(...),
        LocalMulti | AssignMulti { names, values } if eq_len =>
            zip-per-position record_alias_binding,
        _ => false,
    }
}

// In lower():
for stmt in chunk: process_alias_stmt(stmt, ..., false);   // Round 1
let mut changed = true;
while changed {
    changed = false;
    for stmt in chunk {
        if process_alias_stmt(stmt, ..., true) { changed = true; }
    }
}
```

The 8 arms collapse to: 1 helper + 1 dispatch + 1 Round 1 loop +
1 Round 2 fixed-point. Future extensions (e.g. `g = a:m`
MethodCall rebind) add one branch in the helper, not 8 arms.

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First (Codex critical)**: helper
  extract collapses 8 arms.
- [x] **#2 TDD (Codex critical)**: conditional-flow negative pin
  added.
- [x] **#3 FP**: helper is pure with `&mut alias_map`; returns
  bool. Round dispatch external; insert-only flag local.
- [x] **#4 CA**: HIR-only. `src/codegen/`, `src/cli/`,
  `src/pipeline.rs`, `src/parser/`, `src/lexer/` zero-diff.
- [x] **#5 Security**: documented carry-over from ADR 0098
  source-order semantics. No new unsafe paths.
- [x] **#6 Documentation (Codex critical)**: strong §Non-goals
  boundary language — top-level-only / control-flow-non-supported
  / call-before-assign-unresolved.

## Test count delta

```
Step 0:  1056 → 1058 (2 Red + 1 regression-pin always-green +
                       1 negative pin already-Green Day 0)
Step 1:  1056 → 1060 (helper extract + Assign arms; 2 Red Green)
Step 2:  1056 → 1060 (clippy + fmt)
Step 3:  1056 → 1060 (docs only)

Final: 1056 → 1060 green, single atomic commit
  feat(hir,docs): re-assignment alias via Assign arm + helper extract (ADR 0100)
```

## Verification

- `cargo test --no-fail-fast` → **1056 → 1060**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/codegen/ src/cli/ src/pipeline.rs src/parser/ src/lexer/` → **0**
- Manual smoke:
  ```bash
  echo 'local app = {}
  function app.dummy(x) return x end
  function app.format(name) return "hi " .. name end
  local g = app.dummy
  g = app.format
  print(g("world"))' > /tmp/r.lua
  cargo run --quiet -- compile /tmp/r.lua && /tmp/r   # → hi world
  ```

## Future work

- **Control-flow aware refinement** — `if cond then g = a.b.m1
  else g = a.b.m2 end; g(x)` requires control-flow merge or
  conservative skip.
- **Call-before-assign source-order resolution** — pin each call
  site to its preceding alias state.
- **Function-body re-assignment** — nested-scope alias_map.
- **Method-call rebind via Assign** — `g = a:m`.
- **Block-scoped scope tracking**.

## ADR number / phase tag

ADR 0100 = Re-Assignment Alias via StmtKind::Assign Extension +
Helper Extract. Phase tag: `2.6+-reassign-alias` under existing
`2.6+ tables / metatables` sub-lane. Closes ADR 0098/0099
future-work for top-level re-assignment case.
