# Roadmap Rebuild — 2026-07-02 (layer-based sizing + N4 status)

- **Trigger**: Data from the arc between 06-27 and 07-02 (10 N4 sub-ADRs landed at ~1/session pace) validates the 06-27 sub-task decomposition principle but reveals a new signal: **runtime-only sub-tasks fit 1 session; HIR-layer sub-tasks do not**. The N4-F arc (Fs 2b/2c) uncovered four *hidden* structural blockers only visible after implementation attempt.
- **State snapshot**: commit `a4625a7` (ADR 0287, tests 1765).
- **Prior rebuild**: [`roadmap-2026-06-27-rebuild.md`](roadmap-2026-06-27-rebuild.md). Principles 1-7 all carry forward; principle 8 added below.

## What the 06-27 → 07-02 arc taught us

10 N4 sub-ADRs landed in the ~5 days between the 06-27 rebuild and today:

| Sub-task | ADR | Layer | Sessions | Outcome |
|---|---|---|---|---|
| N4-A | 0278 | codegen wire | 1 | ✅ shipped |
| N4-B | 0279 | runtime (bridge) | 1 | ✅ shipped |
| N4-C | 0280 | runtime | 1 | ✅ shipped |
| N4-D | 0281 | runtime + codegen extern | 1 | ✅ shipped |
| N4-E | 0282 | runtime | 1 | ✅ shipped |
| N4-F-1 | 0283 | runtime + codegen | 1 | ✅ shipped (init form) |
| N4-F-2 design | 0284 | docs | (partial) | ✅ decomposed to 2a/2b/2c |
| N4-F-2a | 0285 | parser | 1 | ✅ shipped (parser only) |
| N4-F-2b | — | HIR refactor | ≥ 2 | ❌ still deferred |
| N4-F-2c | — | HIR closure synth | ≥ 2 | ❌ still deferred |
| N4-G | 0286 | runtime + codegen | 1 | ✅ shipped (string repl) |
| N4-H | 0287 | runtime | 1 | ✅ shipped |

### Signal 1: sub-task decomposition works

10 concrete shipped sub-ADRs in the arc validate principle 5 (06-27). `/goal N4-<letter> done` reliably converges in 1 turn when the sub-task is single-layer.

### Signal 2: layer matters

Every completed sub-task above touched either the bridge runtime, codegen extern wiring, or a light HIR arm addition. Every deferred sub-task requires reshape of an existing HIR abstraction:

- **N4-F-2b** needs `ForGeneric` HIR lowering to accept 1-return iter (candidate filter + multi-assign scaffold both hard-coded for 2-return today).
- **N4-F-2c** needs a mechanism for a builtin to *return* a closure value — no precedent; closures are only allocated from user `function` literals via `HirFunction`.

Attempting either in a single session leaks partial state into other subsystems (multi-assign, candidate matching, ForGeneric codegen) and gets reverted for cleanliness.

### Signal 3: hidden blockers surface only during implementation

The 06-27 doc estimated N4-F at 2 sessions total. Data now shows N4-F-2 is actually 3 sub-sessions (2a done, 2b/2c pinned), because the closure-returning iterator needed:

1. Builtin-returned closure synthesis (HirFunction generation)
2. Generic-for 1-name parser shape (done, 2a)
3. ForGeneric HIR 1-return iter relaxation
4. TaggedValue → Number narrowing HIR primitive (surfaced only when trying to desugar `for x in string.gmatch(s, pat)` to `while string.find + string.sub`)

Estimates need a **layer-multiplier**: base sub-task budget × (1 for runtime, 2 for codegen, 3 for HIR reshape).

## Principle 8 (new): layer-based session sizing

Every sub-task carries a **layer tag** on top of its scope literal:

| Layer | Session multiplier | Examples |
|---|---|---|
| `runtime` | ×1 | Pattern engine additions, libc wrappers, arithmetic emit |
| `codegen-arm` | ×1 | New Builtin arm reusing existing helpers |
| `codegen-helper` | ×1.5 | New emit_* helper (new alloc pattern, new dispatch shape) |
| `parser` | ×1 | Grammar extension via existing StmtKind/ExprKind |
| `hir-arm` | ×1 | New HirStmtKind/HirExprKind arm using existing lowering primitives |
| `hir-refactor` | ×2-3 | Reshape existing abstraction (ForGeneric, Callee, multi-return contract) |
| `abi-new` | ×3+ | New closure / calling-convention path (closure-from-builtin, upvalue-as-binding) |

**Rule**: `/goal <sub-task> done` is only safe when the layer is `runtime` / `codegen-arm` / `parser` / `hir-arm`. For `hir-refactor` / `abi-new`, split into sub-sub-tasks with explicit layer tags BEFORE setting the goal.

## Updated N4 status (post-arc)

| sub | scope | layer | status |
|---|---|---|---|
| N4-A | Pattern class基本 wire | codegen-arm | ✅ ADR 0278 |
| N4-B | Quantifier `*`/`+`/`?` | runtime | ✅ ADR 0279 |
| N4-C | Anchor `^`/`$` | runtime | ✅ ADR 0280 |
| N4-D | Single capture `(...)` | runtime + extern | ✅ ADR 0281 |
| N4-E | Set `[abc]`/`[^abc]`/`[a-z]` | runtime | ✅ ADR 0282 |
| N4-F-1 | `string.find(s, pat, init)` | runtime + codegen | ✅ ADR 0283 |
| N4-F-2a | Generic-for 1-name parser | parser | ✅ ADR 0285 |
| **N4-F-2b** | **ForGeneric 1-return HIR** | **hir-refactor** | **❌ 2-3 sessions** |
| **N4-F-2c** | **`Builtin::StringGmatch` closure** | **abi-new** | **❌ 3+ sessions** |
| N4-G | `string.gsub` string-repl | runtime + codegen | ✅ ADR 0286 |
| N4-H | `%b` balanced + `%f` frontier | runtime | ✅ ADR 0287 |

**Effective N4 completeness**: 8/10 sub-tasks (80%) shipped in 1 arc. Gmatch closure form pinned at ~5-6 sessions accumulated across 2b + 2c because of layer multiplier.

## Recomputed N5/N6/N8/N9 estimates with principle 8

### N5 Coroutine runtime

| sub | layer | old estimate | new estimate |
|---|---|---|---|
| N5-A ucontext + noop create | runtime + codegen-helper | 2-3 | 2-3 (unchanged) |
| N5-B resume trivial body | codegen-helper + abi-new | 1-2 | 3-4 (abi-new: swapcontext + save/restore) |
| N5-C yield | abi-new | 2-3 | 3-4 |
| N5-D status/wrap/running | codegen-arm | 1 | 1 (unchanged) |
| N5-E error propagation in coroutine | hir-refactor | 1-2 | 2-3 |

**N5 revised total**: 11-15 sessions (vs 7-11 in 06-27).

### N6 load/require runtime

Same principle: any sub-task involving runtime JIT + closure-from-runtime is `abi-new` layer.

| sub | layer | old estimate | new estimate |
|---|---|---|---|
| N6-A embedded parser | codegen-helper | 2-3 | 2-3 |
| N6-B embedded HIR + JIT | abi-new | 3-5 | 5-8 |
| N6-C `load()` Lua-callable | abi-new | 1-2 | 3-4 (closure-from-runtime = same blocker as N4-F-2c) |
| N6-D `package.path` resolver | codegen-arm | 1 | 1 |
| N6-E `require()` | codegen-helper | 1-2 | 2-3 |
| N6-F `package.loaded` etc. | codegen-arm | 1 | 1 |

**N6 revised total**: 14-20 sessions.

**Thesis re-flag**: N6 is genuinely the largest ADR chain. The AOT-vs-dynamic-loading deviation option (documented in 06-27) becomes more compelling given the revised estimate. Recommend N6 decision be made BEFORE any N6-B attempt.

### N8 Performance

Unchanged from 06-27 (benchmarks + inlining + escape analysis). Layer tags:

| sub | layer |
|---|---|
| N8-A bench harness | codegen-helper |
| N8-B inlining | hir-refactor |
| N8-C escape analysis | hir-refactor |
| N8-D JIT decision-only | docs |

Total 6-9 sessions (was 6-9).

### N9 Multi-target CI

Unchanged. All sub-tasks are infra + codegen-arm layer.

Total 4-6 sessions.

## Updated `/goal` safety table

| `/goal` 文字列 | Layer 判定 | 想定 sessions | 安全度 |
|---|---|---|---|
| `N4-A/B/C/D/E/F-1/F-2a/G/H done` | runtime / codegen-arm / parser | 1 | ✅ 実測 shipped |
| `N4-F-2b done` | hir-refactor | 2-3 | ⚠️ 単一 session だと partial-wire risk |
| `N4-F-2c done` | abi-new | 3+ | ❌ 単一 session 不能 |
| `N4-F done` (含 2b/c) | ミックス | 5-6+ | ❌ 絶対 hook 条件にしない |
| `N7-XX done` (増分) | codegen-arm | 1 | ✅ 実測 shipped x16 |
| `N5-A done` | runtime + codegen-helper | 2-3 | ⚠️ 上限気味 |
| `N5-B/C done` | abi-new | 3-4 | ❌ split 必要 |
| `N5 done` | ミックス | 11-15 | ❌ 絶対 hook 条件にしない |
| `N6-B done` | abi-new | 5-8 | ❌ split 必要 |
| `N6 done` | ミックス | 14-20 | ❌ 絶対 hook 条件にしない |
| `N9-A/B/C/D done` | infra + codegen-arm | 1-2 | ✅ 安全 |
| `N10 done` | docs | 0.5 | ✅ 安全 |

## Recommended next-session picks

Given the layer principle, prioritise `runtime` / `parser` / `codegen-arm` work that fills genuine spec gaps:

1. **N9-A/B/C/D** — CI matrix activation. Infrastructure; 1-2 sessions each; totally parallel to N4/N5/N6.
2. **N7 remaining** — `table.pack/sort/move`, `debug.*` subset, `io.open`, `xpcall`, `select`, `goto/label`, remaining `utf8.*` (codepoint / char / offset).
3. **N5-A** — the *only* N5 sub-task not gated by abi-new layer (ucontext + noop). Land it as an isolated 2-3 session commit.
4. **N4-F-2b spike** — attempt ForGeneric refactor as an isolated 2-3 session goal *only after* pre-planning the sub-sub-tasks explicitly.

Recommend NOT touching N6 until the AOT thesis decision is made.

## Residual 06-27 principles

All 7 principles from 06-27 still hold. Principle 8 is additive.

Principle 4 (session-shape 不整合を認める) gets sharpened: it's not just about "structural work spans sessions" but "**HIR-layer reshape work always spans sessions**" — the layer tag makes this decidable up-front.

## Metrics for next arc

Primary:
- N4-F-2b sub-sub-task landed / month (if attempted at all)
- N5-A landed
- N9 sub-tasks landed

Secondary:
- N7 sub-ADR landed count
- Test count delta

If a month passes with N7-only progress, revisit layer sizing and consider whether the HIR-layer work is being unnecessarily avoided.

## References

- [roadmap-2026-06-27-rebuild.md](roadmap-2026-06-27-rebuild.md) — prior rebuild (this one supersedes on layer-based sizing).
- [roadmap-2026-06-21-rebuild.md](roadmap-2026-06-21-rebuild.md) — the honest-scorecard rebuild that started the N-series.
- [ADR 0284](../design/0284-string-gmatch-deferred.md) — the concrete decomposition case study for N4-F-2b/c.
- [ADR 0253 revised](../design/0253-lua54-preferred-subset-declaration.md) — honest scorecard.
