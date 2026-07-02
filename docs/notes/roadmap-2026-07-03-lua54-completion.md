# Roadmap — Lua 5.4 Completion Path (2026-07-03)

- **Purpose**: Comprehensive roadmap from the present state (tests 1786, ADR 0292) to declared full Lua 5.4 preferred-subset compliance. Consolidates the N1-N10 view (per [`roadmap-2026-06-21-rebuild.md`](roadmap-2026-06-21-rebuild.md), refined 06-27, 07-02) with the §-by-§ spec gap analysis in [`lua54-conformance-roadmap.md`](lua54-conformance-roadmap.md).
- **State snapshot**: commit `3b1e574` (ADR 0292, tests 1786).
- **Method**: apply layer-based sizing (principle 8, [`roadmap-2026-07-02-rebuild.md`](roadmap-2026-07-02-rebuild.md)) to every remaining item and produce a session count.

## What "Lua 5.4 完全対応" means here

Per ADR 0253 (revised), full conformance to the **preferred subset** the project targets, excluding:

- **C API (§4)** — LuMeLIR is AOT, not embeddable-VM.
- **Stand-alone interpreter (§7)** — AOT compiler, not `lua` CLI.
- **`__metatable` hiding** (Non-goal per ADR 0135).

Everything else in the reference manual §2, §3, §6 is in scope.

## Post-arc status (as of 2026-07-03)

### Shipped since N-series started (roadmap 2026-06-21)

| Area | ADRs | Notes |
|---|---|---|
| N2 Table DFS + closure roots | 0254-0257 | Real GC-freeing works for Table-heavy chunks. |
| N3 metamethods | 0258-0261 | `__close`, `__gc`, `__mode`, userdata `newproxy`. |
| N4 pattern matcher | 0278-0287 | classes / quantifiers / anchors / capture / sets / `find(init)` / `gsub` (string repl) / balanced / frontier. |
| N4-F-2a | 0285 | parser 1-name generic-for (structural prep for gmatch closure). |
| N7 stdlib incremental | 0262-0276, 0288-0292 | 21 sub-ADRs across math / os / utf8 / debug (stub) namespaces. |

Test count: 1617 → 1786 (+169 e2e) in ≈2 weeks.

### Currently deferred

- **N4-F-2b/2c**: `Builtin::StringGmatch` closure-returning iterator (2 hir-refactor + abi-new sub-sessions).
- **N5 coroutine runtime**: 0% shipped.
- **N6 load/require**: 0% shipped. Requires AOT thesis decision.
- **N7 remainder**: varargs / `select` / `xpcall` / `goto`-`::label::` / `table.pack`-`sort`-`move` / `io.open`-`lines` / integer subtype ops / bitwise operators.
- **N8 performance**: bench harness + inlining + escape analysis.
- **N9 multi-target CI**: 0%.
- **N10 conformance declaration**: pending everything above.

## Foundation blockers (must resolve first)

Two items pin many others:

### F1. Varargs `...` (parser + HIR + codegen)

- **Blocks**: `select('#', ...)` / `select(n, ...)`, `table.pack(...)`, `table.unpack(t)`, `string.format(...)` extras, `print(...)` variadic beyond current handling, user function `function(...)` bodies, `{...}` table constructor spread.
- **Scope**: New HIR statement / expression kinds for variadic packing; ABI change so variadic returns propagate through the call chain.
- **Layer**: `hir-refactor` + `abi-new`.
- **Sessions**: 4-6.
- **Sub-decomposition**:
  - F1-A parser accepts `...` in expression + declaration position (1)
  - F1-B HIR variadic value representation — Table-like packed slot (1-2)
  - F1-C codegen call-site expansion (1-2)
  - F1-D `select`, `table.pack`, `table.unpack` builtins (1)

### F2. Integer subtype (arithmetic + literal path)

- **Current state**: ADRs 0209-0214 stashed integer syntax at parse time but silent-demote to f64 at HIR (`ValueKind::Number` = f64 only). `math.type` distinguishes via shape heuristics only.
- **Blocks**: `//` integer floor-div, bitwise `~` `&` `|` `<<` `>>` proper semantics, `math.tointeger` genuine result, `1 == 1.0` value equivalence, precision-preserving arithmetic for integer literals ≥ 2^53.
- **Layer**: `hir-refactor` (add `ValueKind::Integer`) + widespread codegen arm updates.
- **Sessions**: 6-10.
- **Sub-decomposition**:
  - F2-A `ValueKind::Integer` variant + kind lattice (1-2)
  - F2-B literal arithmetic (int + int, int + float promotion) (1-2)
  - F2-C floor-div `//` proper integer result (1)
  - F2-D bitwise ops `~ & | << >> ` (1-2)
  - F2-E `math.tointeger` / `math.type` real integer path (1)
  - F2-F comparison equivalence `1 == 1.0` (1-2)

### F3. Lua-level bitwise operators on integers

- Currently: bitwise metamethods (ADR 0148) route through f64 with UB caveats.
- Blocks: idiomatic Lua that uses `n & 0xFF`, `n << 4`.
- Layer: rides on F2 completion.
- Sessions: absorbed into F2-D estimate.

## Remaining N-series decomposition (post-arc)

### N4 remainder (gmatch closure form)

| sub | scope | layer | sessions |
|---|---|---|---|
| N4-F-2b | ForGeneric 1-return HIR + synthetic closure infrastructure | hir-refactor | 2-3 |
| N4-F-2c | `Builtin::StringGmatch` closure-returning + `utf8.codes` sibling | abi-new | 3+ |

Total: 5-6 sessions.

### N5 coroutine runtime

Per 07-02 layer sizing:

| sub | scope | layer | sessions |
|---|---|---|---|
| N5-A | `ucontext_t` stack alloc + `coroutine.create` value | runtime + codegen-helper | 2-3 |
| N5-B | `coroutine.resume` swapcontext | abi-new | 3-4 |
| N5-C | `coroutine.yield` + resume value plumbing | abi-new | 3-4 |
| N5-D | `coroutine.status` / `wrap` / `running` | codegen-arm | 1 |
| N5-E | error-in-coroutine propagation | hir-refactor | 2-3 |

Total: 11-15 sessions.

### N6 load/require (decision required first)

**Decision**: implement or declare out-of-scope.

**Option A — Implement (14-20 sessions)**:

| sub | scope | layer | sessions |
|---|---|---|---|
| N6-A | embedded parser bridge call | codegen-helper | 2-3 |
| N6-B | embedded HIR + JIT execution (LLVM ExecutionEngine) | abi-new | 5-8 |
| N6-C | `load(chunk)` Lua-callable closure result | abi-new | 3-4 |
| N6-D | `package.path` resolver + fs read | codegen-arm | 1 |
| N6-E | `require(name)` + `package.loaded` cache | codegen-helper | 2-3 |
| N6-F | `package.preload` / `dofile` / `loadfile` | codegen-arm | 1 |

**Option B — Declare out-of-scope** (0.5 session):

- N10 conformance declaration adds: "§6.3 `load`/`require` not implemented — AOT compiler thesis."
- Realistic Lua deployments that need `require` remain incompatible; documented deviation.

**Recommendation**: Option B, unless a specific business driver requires runtime loading. Rationale: N6-B alone consumes 5-8 sessions of infrastructure that has no other consumer.

### N7 remainder (spec-gap closure)

Layer=`codegen-arm` items land 1/session. Order by dependency:

| Item | Depends on | Sessions |
|---|---|---|
| `xpcall(f, handler[, ...])` | F1 (variadic args to `f`) | 1-2 |
| `select('#'/n, ...)` | F1 | 1 |
| `table.pack(...)` | F1 | 1 |
| `table.unpack(t [, i, j])` | F1 (return spread) | 1-2 |
| `table.move(a1, f, e, t)` | none | 1 |
| `table.sort(t [, comp])` | possibly F4-F5 (comparator callback ABI) | 2-3 |
| `goto` / `::label::` | parser + HIR (hir-refactor) | 2-3 |
| `io.open` / `close` / `read` / `write` / `lines` | userdata FILE* + closure iterator | 3-5 |
| `io.input` / `output` / `stderr` / `stdout` | rides on io.open | 1 |
| `_G` / `_ENV` proper | ADR 0154 (hir-refactor) | 3-4 |
| `math.huge` / `pi` / `maxinteger` / `mininteger` (constants) | ADR 0208 partial | 1 |
| `__name` / display-name in errors | codegen-arm | 1 |
| Real GC freeing (out of v1 safety mode) | none (design done ADR 0159/0161) | 2-3 |
| Long string `[[...]]` + long comments (probe first) | parser | 0.5-1 |
| Return-mid-block (probe) | probe first | 0.5-1 |

Total: 18-32 sessions.

### N8 performance

Unchanged from 07-02 rebuild:

| sub | layer | sessions |
|---|---|---|
| N8-A bench harness | codegen-helper | 1-2 |
| N8-B inlining heuristic | hir-refactor | 2-3 |
| N8-C escape analysis for closure cells | hir-refactor | 2-3 |
| N8-D JIT decision-only ADR | docs | 1 |

Total: 6-9 sessions.

### N9 multi-target CI

Unchanged:

| sub | layer | sessions |
|---|---|---|
| N9-A aarch64-linux cross-compile | infra | 1 |
| N9-B GitHub Actions matrix + qemu | infra | 1-2 |
| N9-C arm64-darwin | infra | 1-2 |
| N9-D release artifact | infra | 1 |

Total: 4-6 sessions. **Parallelisable with everything else — start any time.**

### N10 final conformance declaration

- 0.5 session. Compose after everything else.

## Grand total & phase ordering

### Session budget (mid-point)

| Phase | Sessions |
|---|---|
| F1 varargs | 4-6 |
| F2 integer subtype | 6-10 |
| N4-F-2b/2c (gmatch closure) | 5-6 |
| N5 coroutine | 11-15 |
| N6 (Option B decision) | 0.5 |
| N7 remainder | 18-32 |
| N8 performance | 6-9 |
| N9 multi-target CI | 4-6 |
| N10 declaration | 0.5 |
| **Total (Option B)** | **55-84** |
| **Total (Option A, N6 implemented)** | **69-104** |

At the recent shipping rate (10-15 sub-ADRs / month), Option B is a ~6-9 month project; Option A is 7-11 months.

### Phase order (recommended)

**Phase P1 — Foundation (parallel where safe, ~15-20 sessions)**:
- F1 varargs (4-6)
- F2 integer subtype (6-10)
- N9 CI matrix (4-6, parallel)

**Phase P2 — Blocked-on-P1 stdlib (~10-15 sessions)**:
- N7 items blocked on F1 (`select`, `table.pack`, `xpcall`, `table.unpack`)
- N7 items blocked on F2 (`math.tointeger` real, bitwise proper)

**Phase P3 — Closure-form iterators (~5-6 sessions)**:
- N4-F-2b/2c (gmatch closure)
- `utf8.codes` (sibling of gmatch)
- `string.gmatch` closure form uses the same infrastructure

**Phase P4 — Coroutines (~11-15 sessions, big chunk)**:
- N5-A through N5-E
- Parallelisable with P3 by different subsystems

**Phase P5 — Standalone stdlib (~5-10 sessions)**:
- `io.open` + userdata FILE* (3-5)
- `_G` / `_ENV` proper (3-4)
- goto/label (2-3)
- Real GC freeing (2-3)

**Phase P6 — Performance + declaration (~10-15 sessions)**:
- N8 performance
- N10 declaration

**Phase P7 (conditional) — N6 load/require (14-20 sessions)**:
- Only if AOT thesis is revised.

## Decision points forced

1. **N6 load/require in or out?** — Recommended: out (Option B). Add a Non-goal to ADR 0253.
2. **Integer subtype full or partial?** — Recommended: full (F2 6-10 sessions). Partial (only `math.type`/`tointeger` fixup) leaves bitwise ops broken.
3. **Coroutine implementation choice** — `ucontext_t` (POSIX) vs `swapcontext` vs handwritten asm. Recommended: `ucontext_t` per 07-02 rebuild.
4. **Bench harness scope** — LuaJIT / lua5.4 / LuMeLIR triple compare or LuMeLIR self-baseline only?

## What's already unblocked and low-cost — pick up next

Given the current arc's demonstrated 1-turn cadence on layer=`codegen-arm` / `runtime` items:

1. **N9-A** (aarch64 cross-compile) — infra, parallel to everything, 1 session.
2. **N7 remainder that doesn't need F1** — `table.move`, `math.huge`/`pi`/`maxinteger`/`mininteger` constants (may partially exist), `os.setlocale` 1-arg setter form, `__name` cosmetic. 1 session each.
3. **N9-B** (CI matrix activation) — 1-2 sessions.
4. **F1-A** (parser accept `...`) — 1 session for parser only; unlocks the whole F1 chain.

Recommend starting the next arc with F1-A. Every session after it unblocks 4+ downstream N7 items.

## Metrics / self-check

**Green flags**:
- 1786 tests green, 0 failed.
- Recent 10 arcs all layer=safe items landing 1/session.
- 3 rebuild docs (06-21, 06-27, 07-02, and this one) all converged on similar total-session budgets (60-100 sessions).

**Red flags**:
- N4-F-2b/2c has been "pending" 3 arcs — hir-refactor / abi-new work is being avoided in favor of easy N7 items. Fine while there's runway, but N7 will exhaust and force a hir-refactor session.
- N6 decision has been deferred 3 rebuilds. Should be made before month 3.

## References

- [`roadmap-2026-06-21-rebuild.md`](roadmap-2026-06-21-rebuild.md) — N-series origin.
- [`roadmap-2026-06-27-rebuild.md`](roadmap-2026-06-27-rebuild.md) — sub-task decomposition principles.
- [`roadmap-2026-07-02-rebuild.md`](roadmap-2026-07-02-rebuild.md) — layer-based sizing.
- [`lua54-conformance-roadmap.md`](lua54-conformance-roadmap.md) — §-by-§ spec gap (partially stale; updated view above supersedes).
- [ADR 0253 revised](../design/0253-lua54-preferred-subset-declaration.md) — honest scorecard.
- [ADR 0284](../design/0284-string-gmatch-deferred.md) — N4-F-2 structural blocker case study.
