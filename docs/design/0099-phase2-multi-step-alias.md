# 0099. Phase 2.6+-multi-step-alias: Top-Level Multi-Step Alias Chain Resolution via Fixed-Point alias_map

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-05-16
- **Deciders:** ShortArrow

## Replan provenance

ADR 0098 (`ac417e5`, 2026-05-16) landed top-level name-rebind
refinement via Pass-1.5 `alias_map`. ADR 0098's future-work
section flagged multi-step alias chains as the remaining gap:

```lua
local h = app.utils.format   -- ADR 0098 populates alias_map["h"]
local g = h                  -- value=Ident("h"), NOT Index chain
                             -- alias_map["g"] NOT populated
g("world")                   -- IndirectCallNoCandidates today
```

The ADR 0098 build only tries `extract_index_chain(value)`. For
`local g = h` the value is a bare Ident — chain extraction returns
None; `alias_map["g"]` stays empty; refinement skips.

Codex post-0098 review (6 視点) verdict: **Refactor → Go**. Critical:
- **Incorporate fixed-point into ADR 0098 build phase, NOT a
  separate Call-side helper**. Responsibility = "build top-level
  name resolution closure" — keep in one place.
- **Insert-only monotonic**. Don't update existing alias_map
  entries; only fill in misses. Guarantees finite termination.

## Non-goals (top-of-ADR)

- **Function-body multi-step alias** — `function f() local h =
  a.b.m; local g = h; g(x) end`. Pass-1 walks TOP-LEVEL only.
- **Re-assignment alias** — `local g; g = a.b.m; g(x)`.
- **Cycle detection** — Lua scoping forbids forward-reference
  (`local a = b` before `b` declared); cycles can't form at
  chunk level. Fixed-point converges trivially via insert-only.
- **Block-scoped scope tracking** — `do local h = ... end;
  local g = h` — h not visible after `do/end`, but walker doesn't
  track scope. Documented limitation.
- **Method-call rebind** — `local g = a:m; g(x)` (MethodCall AST).
- **Aliasing chains crossing function_names / alias_map spaces** —
  `local g = some_funcdef_name; local h = g; h(x)`.
- **Codegen changes** — `emit.rs` zero-diff (HIR-only).

## Context

Pre-ADR-0099 `alias_map` build pass:
```rust
let mut alias_map: HashMap<String, FuncId> = HashMap::new();
for stmt in chunk {
    if let StmtKind::Local { name, value } = &stmt.kind {
        if let Some((chain, method)) = extract_index_chain(value)
            && let Some(&fid) = method_funcs.get(&(chain, method))
        {
            alias_map.insert(name.clone(), fid);
        }
    }
    // LocalMulti: same
}
```

Only "Round 1" — direct Index-chain rebinds. Bare Ident rebinds
(`local g = h`) skip.

After this ADR (Codex critical: same build phase):
```rust
// Round 1: Index-chain (unchanged, ADR 0098 behavior preserved)
for stmt in chunk { ... extract_index_chain ... }

// Round 2+: Ident rebind fixed-point (ADR 0099 addition)
let mut changed = true;
while changed {
    changed = false;
    for stmt in chunk {
        // Local: if value = Ident(other) AND alias_map[other] exists
        //         AND !alias_map.contains_key(name) → insert
        // LocalMulti: same per-position
    }
}
```

## Codex 6-視点 fix checklist

- [x] **#1 non-ad-hoc / Tidy First (Codex critical)**: fixed-point
  incorporated into the ADR 0098 build phase as Round 2+.
- [x] **#2 TDD**: 2 happy + 1 separate single-step regression-pin
  (Codex critical).
- [x] **#3 FP (Codex condition)**: insert-only invariant
  (`!alias_map.contains_key(name)` guard). `changed` is local
  termination state.
- [x] **#4 CA**: HIR-only. `src/codegen/`, `src/cli/`,
  `src/pipeline.rs`, `src/parser/`, `src/lexer/` zero-diff.
- [x] **#5 Security**: termination via monotonic insert-only over
  finite top-level local names. Inline-commented + ADR-documented.
- [x] **#6 Documentation**: ADR 0099 / tagged-semantics §8 row /
  AGENTS.md `‣ 2.6+-multi-step-alias` row.

## Termination proof

`alias_map` is keyed by top-level local names — a finite set
bounded by the count of `StmtKind::Local` + per-position
`StmtKind::LocalMulti` bindings in the chunk. Round 2+ inserts
have the guard `!alias_map.contains_key(name)`, so each insert
strictly grows `alias_map`. Termination occurs when no insert
happens during a full chunk pass (`changed == false`).

Worst-case iteration count: bounded by `N - alias_map.len()`
after Round 1, where N is the count of `StmtKind::Local` /
`LocalMulti` bindings. In practice converges in 2-3 iterations
(chain depth rarely exceeds 2 in Lua).

## Test count delta

```
Step 0:  1053 → 1054 (2 Red + 1 always-green regression-pin)
Step 1:  1053 → 1056 (fixed-point Round 2+; 2 Red → Green)
Step 2:  1053 → 1056 (clippy + fmt)
Step 3:  1053 → 1056 (docs only)

Final: 1053 → 1056 green, single atomic commit
  feat(hir,docs): top-level multi-step alias chain resolution via fixed-point alias_map (ADR 0099)
```

## Verification

- `cargo test --no-fail-fast` → **1053 → 1056**
- `cargo clippy --all-targets -- -D warnings` → clean
- `cargo fmt --check` → clean
- `git diff --stat src/codegen/ src/cli/ src/pipeline.rs src/parser/ src/lexer/` → **0**
- Manual smoke:
  ```bash
  echo 'local app = {}
  app.utils = {}
  function app.utils.format(name) return "hi " .. name end
  local h = app.utils.format
  local g = h
  print(g("world"))' > /tmp/m.lua
  cargo run --quiet -- compile /tmp/m.lua && /tmp/m   # → hi world
  ```

## Future work

- **Function-body multi-step alias** — nested-scope alias_map
  for per-function rebinds.
- **Re-assignment alias** — `local g; g = a.b.m; g(x)`.
- **Block-scoped scope tracking** — track Local visibility
  through `do/end` and function bodies.
- **Method-call rebind** — `local g = a:m; g(x)` (MethodCall AST,
  separate from Index chain).
- **Aliasing chains crossing function_names + alias_map spaces** —
  `local g = some_funcdef_name; local h = g; h(x)`.

## ADR number / phase tag

ADR 0099 = Top-Level Multi-Step Alias Chain Resolution via
Fixed-Point alias_map. Phase tag: `2.6+-multi-step-alias` under
existing `2.6+ tables / metatables` sub-lane. Closes the ADR 0098
future-work for multi-step Ident → Ident aliasing at top-level
chunk scope.
