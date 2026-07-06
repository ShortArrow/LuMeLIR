# LuMeLIR Design Notes

This directory collects **Architecture Decision Records (ADR)** for LuMeLIR.
Each file captures a single design decision — what was chosen, what was rejected, and why.

## Where to look

| Document | Role |
|---|---|
| `docs/design/README.md` (this file) | Global ADR conventions + chronological index |
| `docs/design/NNNN-*.md` | Canonical single-decision records |
| `docs/design/tagged-semantics.md` | TaggedValue runtime SoT (slot layout, producer/consumer matrix, §8 TaggedValue-related ADR appendix) |
| `CONTRIBUTING.md` | Universal working conventions (humans + LLMs) — *current rules* |
| `AGENTS.md` | LLM-agent safety guardrails |

The ADR docs are the source of truth for *what was decided* and *why*; `CONTRIBUTING.md` is the source of truth for *the current rules* derived from those decisions. The two are linked: each policy section in `CONTRIBUTING.md` cross-references the foundational ADR that justifies it.

## When to write an ADR

Write an ADR when **all three** apply:

1. **Defensible alternative existed.** You chose A over B (or C). A future reviewer might ask "why not B?".
2. **Future-binding.** The decision constrains how future code is shaped — layering, ABI, dispatcher contract, dependency boundary, etc.
3. **Not implied by existing decisions.** If an established pattern already covers this case, this is implementation, not architecture.

### Self-test before writing

- *"6 months from now, will reading this doc explain WHY the codebase is shaped this way?"* — Yes → ADR.
- *"In code review, if someone proposed the alternative, would this doc be the canonical rejection record?"* — Yes → ADR.

### Not ADR-worthy — go to commit log instead

- Feature implementation following an established pattern (new builtin, new operator).
- Pure refactor / Tidy First with a single obvious path.
- Bug fix.
- Test or lint policy tweak.

### Project-policy decisions

Foundational engineering policies — FP / CA / TDD / TidyFirst / CI-CD / Release / Security / Documentation / Dependencies / Phase tags / Commit conventions / PR discipline — are recorded as **foundational ADRs** (`0120`–`0131`). `CONTRIBUTING.md` holds the *current rules*; foundational ADRs hold the *rationale and alternatives*. The two are not redundant: `CONTRIBUTING.md` is mutable as practice evolves; the ADR is the immutable record of what was considered and decided.

## `Kind:` header convention

Every ADR file declares its kind at the top:

| Kind | Use |
|---|---|
| `Architecture Decision` | Real ADRs: defensible alternative + future-binding + cross-cutting. Listed under "Architecture Decisions" in the index. |
| `Feature Memo` | Feature-implementation memos from before the criteria was formalized. Kept for history; not referenced as policy. |
| `Refactor Memo` | Pure-refactor records from before the criteria was formalized. Kept for history. |

New ADR files written from now on should normally be `Architecture Decision`. `Feature Memo` / `Refactor Memo` are essentially historical; new feature or refactor work belongs in commit messages, not new ADRs.

## Conventions

### Filename
`NNNN-kebab-title.md`, zero-padded, monotonically increasing.

### ADR ID vs phase tag
- **ADR ID is chronological** — the next ADR always takes the next free integer.
- **Phase tag is semantic** — describes which lane the decision belongs to.
- The two are **not 1:1**. A single phase lane can contain many ADRs (e.g. `2.7r-stdlib-table` covers 0106 / 0107 / 0108 / 0111 / 0118), and one ADR can introduce a new lane.

### Phase tag taxonomy
- **`2.X[a-z]-domain-feature`** — Phase 2 feature lane. `X` is a Phase 2 subphase (0–9), letters extend within. Examples:
  - `2.6a-arr-*` — array tables
  - `2.6b-hash-*` — hash-keyed tables
  - `2.6c-tag-*` — TaggedValue
  - `2.7q-stdlib-math`, `2.7q-stdlib-string` — math / string namespaces
  - `2.7r-stdlib-table` — table namespace
  - `2.7t-stdlib-arg-kind-validation` — cross-cutting validation policy
  - `2.7v-stdlib-string-char` — separated lane for ADR 0113
  - `2.7x-stdlib-io` — io namespace (write + read pair)
  - `2.8e-iter-*` — iteration protocols
  - `2.8f-run-arg-table` — CLI surface
- **`2.devinfra-*`** — cross-cutting Tidy First / dev-infrastructure. Examples:
  - `2.devinfra-emit` (ADR 0090)
  - `2.devinfra-stdout-fwrite` (ADR 0117)
  - `2.devinfra-run-modes` (CLI run-modes commit)
- **No "Phase 3" entries yet** — globals / REPL / metatables / patterns runtime / pcall are all out of scope.

### One decision per file
If a later ADR supersedes an earlier one, add a `Supersedes:` / `Superseded-by:` header rather than editing history. Prefer **annotation over deletion** for the same reason: see "Resolved future-work" below.

### Resolved future-work cleanup style
When a "Future work" bullet in an older ADR is delivered by a later ADR, append `**RESOLVED by ADR XXXX (date)**` inline; do not delete the bullet. Example:

```markdown
- **MLIR shape regression tests** for string object layout.
- **ADR 0113 = `string.char(...)` proper** — direct payoff.
  **RESOLVED by ADR 0113 (2026-05-19)**.
```

History preservation matters for context — future readers should see what the original ADR considered next, even when those follow-ons have landed.

### Decision tense
Write in the present tense of the decision ("We choose X"), not future plans ("We will decide X").

## ADR Template

```markdown
# NNNN. <Title>

- **Status:** Proposed | Accepted | Superseded by NNNN | Deprecated
- **Kind:** Architecture Decision
- **Date:** YYYY-MM-DD
- **Deciders:** <names / handles>

## Context

What problem forced a decision? What constraints apply?

## Decision

The choice made, stated plainly.

## Alternatives Considered

Each rejected option + the reason it was rejected.

## Consequences

What becomes easier, harder, or locked-in as a result.

## Documentation updates

`docs/design/tagged-semantics.md` is the SoT for the
TaggedValue runtime model (ADR 0068). Any ADR that touches a
TaggedValue producer, consumer, dispatch site, or runtime
invariant **must** record updates as a checklist below:

- [ ] §1 slot layout
- [ ] §2 producer / source taxonomy
- [ ] §3 consumer coverage matrix
- [ ] §4 LIC consolidation
- [ ] §5 runtime tag invariants
- [ ] §7 open questions
- [ ] §8 ADR index
- [ ] No `tagged-semantics.md` change required (briefly justify why)
```

Recent ADRs (0080+) adopt a richer working template — Context → Codex critical fixes baked in → Non-goals → Goals → Lua spec compliance → 設計 → Reuse table → Codex 6-視点 checklist → Test count delta → TDD steps → New tests → Verification → Critical files → Risks → Future work → ADR number / phase tag. Both the minimal and the richer template are acceptable; pick what fits the decision's complexity.

## Consistency checks

The cargo test target `tests/adr_doc_consistency.rs` enforces:
- `docs/design/NNNN-*.md` numbering is dense from 0001 to max(found) — no gaps.
- Every `(ADR NNNN)` reference in `AGENTS.md` points at an existing ADR doc.

Run via `cargo test --test adr_doc_consistency`. CI fails fast if a feature commit lands without its ADR doc.

## Index

The index is split by `Kind:` (see "Kind: header convention" above). Architecture Decisions are the primary reading list; Feature and Refactor Memos are kept for history.

### Architecture Decisions

Foundational + decision-grade entries. Read these to understand *why* the codebase is shaped the way it is.

- [0001 — lexer-implementation](0001-lexer-implementation.md)
- [0002 — lib-rs-layering](0002-lib-rs-layering.md)
- [0003 — error-handling](0003-error-handling.md)
- [0004 — parser-implementation](0004-parser-implementation.md)
- [0005 — mlir-environment](0005-mlir-environment.md)
- [0006 — phase1-codegen](0006-phase1-codegen.md)
- [0068 — phase2-6c-tag-doc-consolidate](0068-phase2-6c-tag-doc-consolidate.md) — tagged-semantics.md SoT
- [0082 — phase2-5x-callee-dispatch](0082-phase2-5x-callee-dispatch.md)
- [0083 — phase2-5c-full-closures](0083-phase2-5c-full-closures.md) — supersedes 0044
- [0091 — phase2-callee-normalization](0091-phase2-callee-normalization.md)
- [0092 — phase2-method-syntax](0092-phase2-method-syntax.md)
- [0093 — phase2-method-arg-refine](0093-phase2-method-arg-refine.md)
- [0094 — phase2-method-idx-call-refine](0094-phase2-method-idx-call-refine.md)
- [0096 — phase2-multi-segment-method-def](0096-phase2-multi-segment-method-def.md)
- [0097 — phase2-multi-seg-call-refine](0097-phase2-multi-seg-call-refine.md)
- [0098 — phase2-name-rebind-refine](0098-phase2-name-rebind-refine.md)
- [0099 — phase2-multi-step-alias](0099-phase2-multi-step-alias.md)
- [0100 — phase2-reassign-alias](0100-phase2-reassign-alias.md)
- [0120 — engineering-principles-fp-first](0120-engineering-principles-fp-first.md) — foundational
- [0121 — layering-clean-architecture](0121-layering-clean-architecture.md) — foundational
- [0122 — tdd-red-green-refactor](0122-tdd-red-green-refactor.md) — foundational
- [0123 — tidyfirst-refactor-discipline](0123-tidyfirst-refactor-discipline.md) — foundational
- [0124 — ci-cd-policy](0124-ci-cd-policy.md) — foundational
- [0125 — release-procedure](0125-release-procedure.md) — foundational
- [0126 — security-policy](0126-security-policy.md) — foundational
- [0127 — documentation-policy](0127-documentation-policy.md) — foundational
- [0128 — dependency-addition-policy](0128-dependency-addition-policy.md) — foundational
- [0129 — phase-tag-convention](0129-phase-tag-convention.md) — foundational
- [0130 — commit-message-convention](0130-commit-message-convention.md) — foundational
- [0131 — pr-discipline-code-review](0131-pr-discipline-code-review.md) — foundational
- [0132 — ci-image-base-distro](0132-ci-image-base-distro.md) — foundational follow-up
- [0133 — phase2-completion-criteria](0133-phase2-completion-criteria.md) — Phase 2 scope freeze
- [0134 — metatables-index-read](0134-metatables-index-read.md) — Metatables `__index` Table read-path
- [0135 — metatables-newindex-write](0135-metatables-newindex-write.md) — Metatables `__newindex` Table write-path
- [0136 — raw-set-get-builtins](0136-raw-set-get-builtins.md) — `rawset` / `rawget` builtins (hash key only)
- [0137 — raw-equal-len-builtins](0137-raw-equal-len-builtins.md) — `rawequal` / `rawlen` builtins (Table only)
- [0138 — setmetatable-nil-clear](0138-setmetatable-nil-clear.md) — `setmetatable(t, nil)` metatable clear
- [0139 — taggedvalue-key-newindex-wiring](0139-taggedvalue-key-newindex-wiring.md) — TaggedValue-key IndexAssign routes through `__newindex` helper
- [0140 — metatable-field-hiding](0140-metatable-field-hiding.md) — `__metatable` field hiding (getmetatable + setmetatable protection)
- [0141 — anon-fn-indexassign-param-refine](0141-anon-fn-indexassign-param-refine.md) — Anonymous `FunctionExpr` param-kind refinement from top-level `IndexAssign` sites
- [0142 — tostring-metamethod](0142-tostring-metamethod.md) — `__tostring` metamethod for `tostring(t)`
- [0143 — concat-metamethod](0143-concat-metamethod.md) — `__concat` metamethod for `Table .. Table`
- [0144 — comparison-metamethods](0144-comparison-metamethods.md) — `__eq` / `__lt` / `__le` for Table-Table
- [0145 — gc-strategy](0145-gc-strategy.md) — Phase 2 leak, Phase 3 mark-and-sweep trigger (decision-only)
- [0146 — call-metamethod](0146-call-metamethod.md) — `__call` metamethod (callable Table)
- [0147 — arith-metamethods](0147-arith-metamethods.md) — Arithmetic metamethods (`__add` etc. + `__unm`) for Table
- [0148 — bitwise-metamethods](0148-bitwise-metamethods.md) — Bitwise metamethods (`__band` etc. + `__bnot`) for Table
- [0149 — len-metamethod](0149-len-metamethod.md) — `__len` metamethod for `#t`
- [0150 — index-function-form](0150-index-function-form.md) — `__index = Function` form (static-String key, Number return)
- [0151 — newindex-function-form](0151-newindex-function-form.md) — `__newindex = Function` form (static-String key, Number value)
- [0152 — string-format](0152-string-format.md) — `string.format` minimum subset (`%d` / `%s` / `%f` / `%%`)
- [0153 — pcall-error-strategy](0153-pcall-error-strategy.md) — `pcall` / `error` value propagation (Phase 3 trigger, decision-only)
- [0154 — env-true-globals-strategy](0154-env-true-globals-strategy.md) — `_ENV` / true globals (Phase 3 trigger, decision-only)
- [0155 — string-patterns-strategy](0155-string-patterns-strategy.md) — `string` patterns (Phase 3 trigger, decision-only)
- [0156 — gc-architecture-v1](0156-gc-architecture-v1.md) — Phase 3 GC architecture v1 (data structures + algorithm; decision-only)
- [0157 — gc-allocator-wrapper](0157-gc-allocator-wrapper.md) — Phase 3 GC step 1: `emit_gc_alloc` + string-obj migration + `collectgarbage("count")`
- [0158 — gc-migrate-remaining-types](0158-gc-migrate-remaining-types.md) — Phase 3 GC step 2: migrate 8 remaining allocator sites
- [0159 — gc-mark-phase](0159-gc-mark-phase.md) — Phase 3 GC step 3: mark phase design (decision-only)
- [0160 — gc-stack-walk](0160-gc-stack-walk.md) — Phase 3 GC step 4: alloca-slot stack walk (decision-only)
- [0161 — gc-sweep-phase](0161-gc-sweep-phase.md) — Phase 3 GC step 5: sweep phase design (decision-only)
- [0162 — gc-auto-trigger](0162-gc-auto-trigger.md) — Phase 3 GC step 6: 1 MiB automatic trigger (decision-only)
- [0163 — gc-finaliser](0163-gc-finaliser.md) — Phase 3 GC step 7: `__gc` finaliser hook (decision-only)
- [0164 — gc-weak-tables](0164-gc-weak-tables.md) — Phase 3 GC step 8: weak tables `__mode` (decision-only)
- [0165 — number-key-index-array-oob-fallback](0165-number-key-index-array-oob-fallback.md) — Single-hop Table-form `__index` fallback for Number-key array OOB (closes ADR 0134 test 7)
- [0166 — index-function-form-number-key](0166-index-function-form-number-key.md) — Function-form `__index` for Number key (ADR 0150 sibling)
- [0167 — multi-hop-number-key-index](0167-multi-hop-number-key-index.md) — Multi-hop Table-form `__index` for Number key (ADR 0134 depth-recursion mirror)
- [0168 — newindex-number-key-table-form](0168-newindex-number-key-table-form.md) — Single-hop Table-form `__newindex` for Number key (key > length case)
- [0169 — newindex-function-form-number-key](0169-newindex-function-form-number-key.md) — Function-form `__newindex` for Number key (ADR 0151 mirror, ADR 0166 sibling)
- [0170 — multi-hop-number-key-newindex](0170-multi-hop-number-key-newindex.md) — Multi-hop Table-form `__newindex` for Number key (ADR 0167 mirror)
- [0171 — newindex-mid-array-nil-trigger](0171-newindex-mid-array-nil-trigger.md) — Mid-array `TAG_NIL` slot also triggers Number-key `__newindex`
- [0172 — rawset-number-key](0172-rawset-number-key.md) — `rawset(t, n, v)` with Number key (ADR 0136 deferral, write side only)
- [0173 — rawget-number-key](0173-rawget-number-key.md) — `rawget(t, n)` with Number key (TaggedValue out-slot, OOB → nil)
- [0174 — rawget-tagged-key](0174-rawget-tagged-key.md) — `rawget(t, k)` with `Local(TaggedValue)` key (runtime tag dispatch)
- [0175 — rawset-tagged-key](0175-rawset-tagged-key.md) — `rawset(t, k, v)` with `Local(TaggedValue)` key, non-TaggedValue value
- [0176 — rawset-tagged-key-tagged-value](0176-rawset-tagged-key-tagged-value.md) — Full pairs-body `rawset(t, k, v)` with both key + value `Local(TaggedValue)`
- [0177 — index-tagged-key](0177-index-tagged-key.md) — `t[k]` with `Local(TaggedValue)` key on the tagged-consumer path (`local x = t[k]`)
- [0178 — tagged-key-runtime-dispatch-helper](0178-tagged-key-runtime-dispatch-helper.md) — Tidy First: extract the shared TaggedValue-key tag-dispatch scaffold used by 0174 / 0175 / 0176 / 0177
- [0179 — non-local-tagged-source-materialisation](0179-non-local-tagged-source-materialisation.md) — HIR materialises non-Local TaggedValue (Call-return) source into a synth local at RawGet/RawSet arg positions, resolving the Local-only restriction in ADRs 0174/0175/0176
- [0180 — param-table-context-inference](0180-param-table-context-inference.md) — Function parameters inferred as `Table` when used in pairs/ipairs/Index/MethodCall/table-consumer-builtin contexts; symmetric to the existing `Function(arity)` body-walk inference
- [0181 — param-string-context-inference](0181-param-string-context-inference.md) — Function parameters inferred as `String` when used as Concat operand or `string.*` method first arg; direct mirror of 0180 for the String kind
- [0182 — param-inference-kind-parameterised-helpers](0182-param-inference-kind-parameterised-helpers.md) — Tidy First refactor of the 0180/0181 body-walker: collapses `mark_ident_as_table`/`mark_ident_as_string` into one `mark_ident_as` helper and extracts `is_body_decisive_kind` predicate
- [0183 — methodcall-string-receiver-dispatch](0183-methodcall-string-receiver-dispatch.md) — `s:upper()` and other `string.<method>` method-syntax forms dispatch via the namespace builtin chokepoint when the receiver is `ValueKind::String`; inference fold-through completes ADR 0181's MethodCall future-work
- [0184 — gc-type-meta-and-size-guard](0184-gc-type-meta-and-size-guard.md) — Tidy First prep for Phase 3 GC mark phase: `gc_type_meta(type_tag)` decision table in `tagged.rs` + ≥ 4 GiB payload guard in `emit_gc_alloc`; consumes R2/R3 from the 0159-0162 pre-flight review
- [0185 — gc-mark-sweep-v1-safety-mode](0185-gc-mark-sweep-v1-safety-mode.md) — `emit_gc_mark` + `emit_gc_sweep` MLIR helpers implementing ADR 0159's v1 safety-mode subset and ADR 0161's sweep algorithm verbatim; `collectgarbage()` rewired through them; worklist + DFS deferred to ADR 0187
- [0186 — gc-auto-trigger-and-func-factoring](0186-gc-auto-trigger-and-func-factoring.md) — implements ADR 0162's 1 MiB auto-trigger inside `emit_gc_alloc`, factors the ADR 0185 inline walks into `llvm.func @gc_mark` / `@gc_sweep`, and adds the post-sweep threshold doubling (capped at 1 GiB)
- [0187 — indexassign-value-side-tagged-source](0187-indexassign-value-side-tagged-source.md) — closes the IndexAssign value-side TaggedValue gap (codegen `unreachable!` at `emit.rs:3980`): HIR materialises non-Local TaggedValue values via the existing ADR 0179 helper; codegen adds a `ValueKind::TaggedValue` arm to the static-key match symmetric to ADR 0138-M's TaggedValue-key arm
- [0188 — non-local-tagged-source-residuals](0188-non-local-tagged-source-residuals.md) — closes 4 residual non-Local TaggedValue source gaps surfaced by a systematic audit: RawSet value (any key), Index/IndexTagged read key, IndexAssign key. All routed through the existing `materialize_tagged_source_if_needed` chokepoint — codegen `UnsupportedExpr` paths become defensive guards behind a HIR invariant
- [0189 — phase3-entry-criteria](0189-phase3-entry-criteria.md) — Phase 3 entry / scope freeze meta-ADR (mirror of ADR 0133's Phase 2 pattern): Phase 3 = PRD Domain Specific Features (Rust-Lua Bridge + Embedded register-ops dialect); orders Bridge first per PRD memo; Phase 2 epilogue items (GC stack walk, pcall, _ENV, string patterns) run in parallel
- [0190 — phase2-epilogue-close-decisions](0190-phase2-epilogue-close-decisions.md) — defers all 4 Phase 2 epilogue items (GC actual freeing, pcall, _ENV, string patterns) to Phase 4; satisfies ADR 0189 §3 close criterion for Phase 3
- [0191 — rust-lua-bridge-mvp](0191-rust-lua-bridge-mvp.md) — Rust-Lua Bridge MVP: `rust.add(a, b)` namespace dispatch + `build.rs` compiles `src/bridge_runtime.rs` to a bundled object + `cc` link integration; satisfies ADR 0189 §1 close criterion (Bridge end-to-end)
- [0192 — embedded-register-ops-entry](0192-embedded-register-ops-entry.md) — Embedded register-ops dialect entry decision (decision-only; pins abstract MVP surface volatile load/store + bit-field rmw; target selection deferred until embedded user emerges); satisfies ADR 0189 §2 close criterion
- [0193 — phase4-entry-criteria](0193-phase4-entry-criteria.md) — Phase 4 entry / scope freeze (post-PRD completeness + optimisation): enumerates workstreams (PRD §7 perf + binary-size metrics, epilogue items, Bridge sub-pieces, Embedded impl) and ordering; benchmark harness identified as first Phase 4 implementation ADR
- [0194 — benchmark-harness-mvp](0194-benchmark-harness-mvp.md) — Phase 4 benchmark harness MVP: recursive `fib(30)` micro-benchmark with LuMeLIR-vs-LuaJIT wall-clock comparison (median of 5 iterations, ratio reported, no perf assertion); satisfies ADR 0193 §Ordering §1
- [0195 — phase5-entry-criteria](0195-phase5-entry-criteria.md) — Phase 5 entry / scope freeze (Coroutines + Runtime Eval `load`/`require` + Production Hardening); depends on Phase 4 closing; aggregate 13-21 ADRs / 16-27 sessions estimated
- [0196 — integer-float-subtype-design](0196-integer-float-subtype-design.md) — Phase 4a top-priority design entry: introduce `ValueKind::Integer` (i64) alongside existing `Number` (f64) per Lua 5.4 §2.1; 10 sub-ADRs (0197-0206) cover parser / HIR / codegen / tagged ABI / cross-subtype semantics in three migration phases (additive → opt-in → strict)
- [0197 — integer-literal-token-additive](0197-integer-literal-token-additive.md) — Integer/Float Phase A additive: lexer adds `TokenKind::Integer(i64)` distinguishing `42`/`0xFF` from `42.0`/`1e5`; parser silently demotes to existing f64 path so 1431 tests stay green; first implementation step of ADR 0196 arc
- [0198 — next-arity-1](0198-next-arity-1.md) — `next(t)` arity 1 per Lua 5.4 §6.1: `Builtin::Next` arity (2,2) → (1,2); HIR synthesizes nil for arg 2 when omitted; resolves bucket E §E8 from the leftover roadmap
- [0199 — table-constructor-keyed-fields](0199-table-constructor-keyed-fields.md) — Lua 5.4 §3.4.9 bracket-key `{[k]=v}` and named-key `{name=v}` table constructor field forms; AST/HIR `TableField` enum; keyed fields desugar via IndexAssign pre-statements; resolves bucket E §E6 from the leftover roadmap
- [0200 — collectgarbage-setpause](0200-collectgarbage-setpause.md) — Lua 5.4 §6.1 `collectgarbage("setpause", n)`: relax arity to (0,2); new `g_gc_pause` mutable global (init 200); ADR 0186 doubling logic uses `g_gc_pause / 100` instead of hardcoded `*2`; resolves bucket D §0186-setpause
- [0201 — string-reverse](0201-string-reverse.md) — Lua 5.4 §6.4 `string.reverse(s)` byte-wise reversal; new `Builtin::StringReverse` variant routed through ADR 0103's namespace dispatch; resolves one bucket B real gap
- [0202 — string-format-hex](0202-string-format-hex.md) — `string.format` accepts `%x` / `%X` hex specs; extends ADR 0152's MVP set; another bucket B real gap resolved
- [0203 — string-format-octal-general-float](0203-string-format-octal-general-float.md) — `string.format` accepts `%o` octal and `%g` general-float specs; same dispatch shape as ADRs 0152/0202
- [0204 — string-format-scientific-char](0204-string-format-scientific-char.md) — `string.format` accepts `%e` scientific and `%c` char specs
- [0205 — string-format-i-alias](0205-string-format-i-alias.md) — `%i` C99 alias for `%d` in `string.format`; pure alias, no new variant
- [0206 — benchmark-string-concat](0206-benchmark-string-concat.md) — second micro-benchmark in the ADR 0194 harness: `..` in a 200-iter loop; LuMeLIR observed 0.44x faster than LuaJIT (startup-amortized); consumes one bucket C item from ADR 0194 §Future work
- [0207 — benchmark-table-ops](0207-benchmark-table-ops.md) — third micro-benchmark: `table.insert` + sum loop over N=500; LuMeLIR 0.48x faster than LuaJIT; closes ADR 0194 §Future work table-ops item
- [0208 — math-constants](0208-math-constants.md) — `math.pi` / `math.huge` numeric constants recognised by HIR Index lowering's namespace short-circuit; composes with arithmetic + print paths without runtime change
- [0209 — integer-ast-hir-variant](0209-integer-ast-hir-variant.md) — Phase B opt-in of ADR 0196 Integer/Float arc: AST `ExprKind::Integer(i64)` + HIR `HirExprKind::Integer(i64)`; `infer_kind` returns `ValueKind::Number` (silent demotion at kind layer); codegen emits `i64 const + sitofp` to demote at leaf. First implementation step of milestone M1.
- [0210 — math-type-static-shape](0210-math-type-static-shape.md) — `math.type(x)` returns `"integer"` / `"float"` for `HirExprKind::Integer` / `HirExprKind::Number` arg literals, `nil` for other shapes (Phase B subtype loss documented). First user-visible Integer/Float distinction; M1 second sub-ADR.
- [0211 — math-tointeger-static-shape](0211-math-tointeger-static-shape.md) — `math.tointeger(x)` returns the integer form of exactly-representable Integer / Number literals, `nil` for fractional or Locals/Calls (Phase B). Parallel to ADR 0210; M1 third sub-ADR.
- [0212 — math-integer-constants](0212-math-integer-constants.md) — `math.maxinteger` / `math.mininteger` constants lower to `HirExprKind::Integer(i64::MAX/MIN)`; composes with `math.type` to return `"integer"`. M1 fourth sub-ADR.
- [0213 — integer-binop-constant-folding](0213-integer-binop-constant-folding.md) — HIR folds Integer+Integer arithmetic / bitwise / floor-div / mod / shifts to `HirExprKind::Integer`, preserving subtype through static arithmetic so `math.type(1 + 2)` returns `"integer"`. Overflow / div-by-zero fall through to f64 path. M1 fifth sub-ADR.
- [0214 — print-integer-subtype](0214-print-integer-subtype.md) — `print(HirExprKind::Integer)` emits `i64 const` + `printf("%lld")`, preserving precision (`print(math.maxinteger)` → `9223372036854775807`) and dropping the `.0` artifact. New `fmt_lld_raw` cstr global. M1 sixth sub-ADR.
- [0215 — pcall-setjmp-infrastructure](0215-pcall-setjmp-infrastructure.md) — libc `_setjmp`/`longjmp` externs + `g_jmpbuf` (512B mutable) + `g_error_value` (i64 boxed-string ptr) + chunk-level setjmp landing pad in `emit_main`. `error(msg)` rewired to longjmp; uncaught still prints msg + exits 1 (ADR 0033 contract preserved). M2 first sub-ADR; `pcall` consumer in ADR 0216.
- [0216 — pcall-builtin-single-return](0216-pcall-builtin-single-return.md) — `Builtin::Pcall` consumer of the ADR 0215 foundation. `pcall(f)` returns `Bool` — `true` on normal return, `false` on caught `error()`. Scope: arity (1,1), Local of `Function(0)` arg only. Multi-return `(ok, err)` deferred to ADR 0217. M2 second sub-ADR.
- [0217 — pcall-multireturn-abi](0217-pcall-multireturn-abi.md) — Widens `Builtin::Pcall::ret_kinds` to `[Bool, TaggedValue]` and adds `emit_call_pcall_into_locals` for the multi-assign path. `local ok, err = pcall(f)` — err is String(msg) on caught error, Nil on success. Composes with ADR 0021 multi-assign (Next precedent). M2 third sub-ADR — M2 milestone close.
- [0218 — gc-chunk-safe-real-freeing](0218-gc-chunk-safe-real-freeing.md) — First M3 sub-ADR. Adds `chunk_safe_for_real_gc` static predicate + chunk-level String-slot root table. For safe chunks (no Tables / user fns / TaggedValue) the mark phase walks `g_gc_head` and checks each tracked object against the root table; transient string allocations are freed. Unsafe chunks stay in ADR 0185 v1 safety mode. Per-frame stack walk for user fns deferred per ADR 0160.
- [0219 — gc-user-fn-frame-safety](0219-gc-user-fn-frame-safety.md) — Second M3 sub-ADR. Relaxes the chunk-safe predicate to allow non-capturing user functions. Adds `g_gc_fn_depth` re-entrancy counter incremented at fn entry, decremented at exit; mark phase falls back to v1 safety mode while non-zero so allocations live in user fn frames survive. Chunk-level collection after the fn returns observes real freeing.
- [0220 — gc-tagged-value-roots](0220-gc-tagged-value-roots.md) — Third M3 sub-ADR (closing). Relaxes the chunk-safe predicate to allow `ValueKind::TaggedValue` chunk locals via a parallel `g_chunk_tv_table`; mark phase reads slot tag at offset 0 and follows the payload ptr at offset 8 when TAG_STRING. Also fixes pcall + setjmp to save/restore `g_gc_fn_depth` so a caught longjmp doesn't leak the safety-mode fallback.
- [0221 — env-globals-chunk-table](0221-env-globals-chunk-table.md) — First M4 sub-ADR. HIR pre-pass scans the AST for bare `_G` / `_ENV` and prepends `local _G = {}` (and `local _ENV = _G`) so `_G[k] = v` / `_G[k]` ride the existing Table machinery. No codegen change; pure HIR-layer rewrite. Builtin migration and `_ENV` upvalue threading deferred to future M4 sub-ADRs.
- [0222 — env-globals-extended-surface](0222-env-globals-extended-surface.md) — Second M4 sub-ADR. Pins the `_G` extended surface: String values, mixed Number+String across distinct keys, `pairs(_G)` iteration, single-level user-fn read+write capture, and reassignment overwrite semantics. Documents the Bool/Nil-in-Table and multi-hop nested closure capture limitations. No new codegen; 5 e2e regression pins.
- [0223 — env-sandbox-rebind](0223-env-sandbox-rebind.md) — Third M4 sub-ADR (closing). Verifies `local _ENV = {}` shadows the synthesised alias via standard Lua lexical scoping — sandbox writes target the new table; `_G` retains the original chunk-level binding. No new code; 3 e2e regression pins. Closes M4 milestone at 3/3-5 minimum-viable.
- [0224 — bridge-string-marshaling](0224-bridge-string-marshaling.md) — First M5 sub-ADR. Bridge widens to String args via `rust.strlen(s) -> Number`. `extern "C" fn rust_strlen(*const u8)` reads the i64 length header at offset 0 of the boxed-string-object (ADR 0112 layout) — same byte `string.len` reads. No allocator coordination, no ownership transfer. 4 e2e: literal / empty / xcheck vs `string.len` / concat-result.
- [0225 — bridge-bool-marshaling](0225-bridge-bool-marshaling.md) — Second M5 sub-ADR. Bridge handles Bool args + returns via `rust.invert(b) -> Bool`. ABI hop: `llvm.zext i1 → i8` at the call site, `llvm.trunc i8 → i1` on the return — matches the x86_64-linux C ABI for `bool`. Lua method name is `invert` because `not` is reserved. 4 e2e: true / false / round-trip / `if`-predicate.
- [0226 — bridge-multi-arg-mixed-kind](0226-bridge-multi-arg-mixed-kind.md) — Third M5 sub-ADR (closing). Bridge `rust.starts_with(s: String, prefix: String) -> Bool` composes the per-kind ABI shapes from ADRs 0224 + 0225 into a multi-arg mixed-kind signature. The Rust side reads both length headers (ADR 0112 layout) and short-circuits / byte-compares. Closes M5 milestone at 3/2-3 minimum-viable.
- [0227 — m1-m5-roll-up](0227-m1-m5-roll-up.md) — M6 meta-ADR. Records M1-M5 minimum-viable closes (ADRs 0209-0226, 17 sub-ADRs, 1464 → 1519 tests). Declares the Phase 4a language-level subset closed; full Phase 4 close still pending PRD §7 performance + binary-size metrics. Enumerates M-series stretch + M7-M17 remaining work.
- [0228 — string-find-plain](0228-string-find-plain.md) — First M7 sub-ADR. `string.find(s, sub)` literal substring search. Returns TaggedValue Number-or-Nil (1-indexed start position on match). Runtime helper `lumelir_string_find_plain` shares the bridge object. Also adds a non-trapping `if Local(TaggedValue) then ...` codegen path (reads tag at slot offset 0 instead of triggering `emit_value_slot_check_number`). Magic-char patterns + multi-return `(start, end)` deferred to subsequent M7 sub-ADRs.
- [0229 — string-find-multireturn](0229-string-find-multireturn.md) — Second M7 sub-ADR. Widens `Builtin::StringFind::ret_kinds` to `[TaggedValue, TaggedValue]` and adds `emit_call_string_find_into_locals` so `local s, e = string.find(s, sub)` yields both positions. `end = start + sub_len - 1` is computed in MLIR (no Rust-side change). Single-assign keeps the start-only truncation per ADR 0081 Next precedent.
- [0230 — string-match-plain](0230-string-match-plain.md) — Third M7 sub-ADR. `string.match(s, sub)` literal substring match. In plain mode the matched substring equals `sub`; emit arm reuses `lumelir_string_find_plain` for the hit/miss decision then stores `TAG_STRING + sub_ptr` on hit or `TAG_NIL` on miss. Composes with the ADR 0228 non-trapping `if Local(TaggedValue)` cond path. Captures + patterns deferred to the matcher port.
- [0231 — string-pattern-char-classes](0231-string-pattern-char-classes.md) — Fourth (closing) M7 sub-ADR. Bounded pattern matcher core: char classes (`%a` / `%d` / `%s` / `%w` / `%p` / `%l` / `%u` / `%x` / `%c` + complements), `.` wildcard, `%X` magic-char escape. Runtime helper `lumelir_string_match_extents` returns packed i64 (low 32 = start, high 32 = matched byte length). Both `string.find` arms upgraded; `string.match` pattern-aware deferred (needs new String allocation). Closes M7 at 4/4-6 sub-ADRs.
- [0232 — local-number-subtype](0232-local-number-subtype.md) — First M8 sub-ADR. Adds `LocalInfo::subtype: NumberSubtype { Unknown, Integer, Float }` populated by a post-pass over the lowered HIR. `math.type(Local)` now consults the subtype so `local x = 42; math.type(x)` returns `"integer"`. Merge semantics: first seen wins; mismatched RHS widens to Unknown.
- [0233 — local-integer-print-tostring](0233-local-integer-print-tostring.md) — Second M8 sub-ADR. `print(Local-Integer)` + `tostring(Local-Integer)` route through a `%lld` fast path so values like `1000000000000` print as `"1000000000000"` instead of `"1e+12"`. New `emit_tostring_integer` helper mirrors the Number snprintf path with `fptosi` + `fmt_lld_raw`. Float subtype Locals keep the `%g` path unchanged.
- [0234 — local-subtype-propagation](0234-local-subtype-propagation.md) — Third M8 sub-ADR. Widens the `propagate_number_subtype` post-pass classifier so subtype flows through Local reads and integer-preserving BinOps (Add/Sub/Mul/FloorDiv/Mod/Bitwise/Shifts). `Div` and `Pow` always yield Float per Lua §3.4.1; mixed Integer/Float widens to Float; Unknown is contagious.
- [0235 — math-tointeger-local](0235-math-tointeger-local.md) — Fourth (closing) M8 sub-ADR. `math.tointeger(Local-Integer)` returns the integer value: load slot f64, store as TaggedValue Number. Other Local subtypes (Float, Unknown) fall through to Nil. Composes with ADR 0234 so `local b = a * 7; math.tointeger(b)` works. Closes M8 at 4/4-6 sub-ADRs.
- [0236 — const-close-attributes](0236-const-close-attributes.md) — First M9 sub-ADR. Parser accepts `local NAME <const>` / `local NAME <close>` per Lua 5.4 §3.3.7. HIR sets `LocalInfo::is_const` and inserts the Local into `readonly_locals` for the `const` case; subsequent Assign is rejected. `<close>` parses + reaches HIR but its `__close` metamethod dispatch is deferred to M9-C. Unknown attribute names error as `HirError::UnknownAttribute`.
- [0237 — close-readonly-multi-attr](0237-close-readonly-multi-attr.md) — Second M9 sub-ADR. `<close>` widens to the same readonly enforcement as `<const>` per Lua 5.4 §3.3.8. Multi-binding `local NAME <ATTR>, ...` now applies per-name attribute enforcement (both the parallel and single-Call-expansion branches). The `__close` metamethod dispatch is still M9-C scope.
- [0238 — gc-finalizer-field-pin](0238-gc-finalizer-field-pin.md) — First M10 sub-ADR. Pins existing `__gc` metatable-field capability as a regression contract; documents runtime auto-dispatch as M10-stretch (gated on M3-extended Tables-as-chunk-roots). No new code; 3 e2e pin metatable storage + setmetatable acceptance + multi-Table sharing.
- [0239 — gc-mode-field-pin](0239-gc-mode-field-pin.md) — Second (closing) M10 sub-ADR. Pins `__mode = "k"` / `"v"` / `"kv"` metatable-field storage + round-trip; documents weak-table runtime semantics as M10-stretch. Closes M10 at 2/2-3 minimum-viable. Runtime `__gc` dispatch + weak-key/value clearing both gated on M3-extended Table DFS.
- [0240 — math-unary-expansion](0240-math-unary-expansion.md) — First M11 sub-ADR. Adds `math.ceil`, `math.tan`, `math.asin`, `math.acos`, `math.atan` via libm extern declarations sharing the existing f64→f64 dispatch shape (ADRs 0101 / 0102). Bundles 5 functions in one ADR — per-function overhead is one HIR variant + one libm name + one match row.
- [0241 — math-max-min-variadic](0241-math-max-min-variadic.md) — Second M11 sub-ADR. Variadic `math.max(x, ...)` / `math.min(x, ...)` lowered as a left-to-right reduce over `arith.maximumf` / `arith.minimumf`. Shares the variadic-Number per-position kind dispatch with `string.char` (ADR 0113).
- [0242 — os-namespace-mvp](0242-os-namespace-mvp.md) — Third (closing) M11 sub-ADR. New `os` namespace adds `os.time()` (libc `time(NULL)`), `os.clock()` (libc `clock() / 1_000_000`), and `os.getenv(name)` (libc `getenv` with String-or-Nil TaggedValue return). Closes M11 at 3/3-5 minimum-viable.
- [0243 — bridge-error-propagation](0243-bridge-error-propagation.md) — First M12 sub-ADR. `rust.fail(msg)` raises a Lua error from Rust via setjmp/longjmp. Routes through a new external-linkage `lumelir_raise_error(msg_ptr)` codegen helper (the Internal-linkage `g_error_value` / `g_jmpbuf` globals stay invisible to the bridge object). Composes with `pcall` for catchable error flow.
- [0244 — userdata-deferral-pin](0244-userdata-deferral-pin.md) — Second (closing) M12 sub-ADR. Pins the existing "opaque-Number" workaround for Bridge handle returns; documents the full real-userdata implementation as blocked on M3-extended Table DFS. Closes M12 at 2/2-3 minimum-viable.
- [0245 — coroutine-main-thread-introspection](0245-coroutine-main-thread-introspection.md) — First M13 sub-ADR. `coroutine.isyieldable()` returns `false` and `coroutine.running()` returns `(nil, true)` — both spec-conformant from main thread without any runtime. New `coroutine` namespace; multi-assign dispatch via the existing Next / Pcall / StringFind precedent.
- [0246 — coroutine-runtime-deferral](0246-coroutine-runtime-deferral.md) — Second (closing) M13 sub-ADR. Strategy decision for the full coroutine runtime: LLVM coroutine intrinsics preferred long-term, POSIX `ucontext` as a pragmatic fallback. Enumerates the strategy-independent architectural deltas + M13-stretch test plan. Closes M13 at 2/2-3 minimum-viable.
- [0247 — package-table-synthesis](0247-package-table-synthesis.md) — First M14 sub-ADR. HIR pre-pass synthesises a minimal `local package = { config, path, cpath }` Table when the source references `package`. Spec-default POSIX values per Lua 5.4 §6.3. Reuses the ADR 0221 `_G`/`_ENV` synthesis pattern.
- [0248 — load-require-runtime-deferral](0248-load-require-runtime-deferral.md) — Second (closing) M14 sub-ADR. Strategy decision for the full `load` / `require` / `package.loaded` runtime: embedded LuMeLIR pipeline (self-hosting) preferred long-term, with external-bytecode-loader / pre-baked-imports / userdata-module-handle as alternatives. Enumerates architectural deltas + M14-stretch test plan. Closes M14 at 2/2-3 minimum-viable.
- [0249 — unsafe-block-audit](0249-unsafe-block-audit.md) — First M15 sub-ADR. Audits all 31 unsafe sites in `src/`: 6 bridge FFI primitives, 24 melior `Value` lifetime transmutes, 1 String-attribute byte carrier. Each has a SAFETY justification + bounded escalation path. No code changes.
- [0250 — phase4-close-declaration](0250-phase4-close-declaration.md) — Second (closing) M15 sub-ADR. Declares Phase 4 closed per ADR 0193's three close criteria (workstreams landed/deferred + explicit PRD §7 metric gap acceptance). 42 sub-ADRs across 16 milestones (M1-M15) in 6 days; 1431 → 1617 tests; zero regression. M-stretch residual work enumerated.
- [0251 — ci-matrix-audit](0251-ci-matrix-audit.md) — First M16 sub-ADR. Adds a `cargo audit` security-advisory step to CI (`--deny warnings` gates on RustSec advisories at warn-level or above). Documents the multi-target CI matrix plan: x86_64-linux active, aarch64-linux / arm64-darwin / x86_64-windows queued.
- [0252 — phase5-close-declaration](0252-phase5-close-declaration.md) — Second (closing) M16 sub-ADR. Declares Phase 5 closed per ADR 0195's three close criteria. Phase 4 and Phase 5 both closed on 2026-06-21. 44 sub-ADRs (0209-0252) landed across 6 days. Coroutine runtime, load/require runtime, multi-target CI activation, release artifact pipeline all deferred with documented strategies.
- [0253 — lua54-preferred-subset-declaration](0253-lua54-preferred-subset-declaration.md) — Revised 2026-06-21. Honest Lua 5.4 scorecard (3 markers: ✅ shipped / ⏸ pinned / ❌ deferred). 47/~87 (~54%) shipped, not the originally-claimed ~56. Original "preferred subset met" framing retracted.
- [0254 — gc-table-array-propagation](0254-gc-table-array-propagation.md) — First N2 (M3-extended) sub-ADR. Widens `chunk_safe_for_real_gc` to allow `ValueKind::Table` slots; adds a second mark-phase pass that walks each BLACK Table's `array_buf` and marks referenced Strings BLACK via a membership-checked write (avoiding `.rodata` segfaults from static literals). Single-level depth; Tables-in-Tables + hash-part walking deferred to N2-B.
- [0255 — gc-table-hash-and-nested](0255-gc-table-hash-and-nested.md) — Second N2 sub-ADR. Extends the propagation pass with hash-bucket walking (key + value slots × `hash_cap` per Table) and TAG_TABLE element propagation (Tables-in-Tables). The whole pass wraps in a fixed 8-iteration outer loop so nested Tables up to 8 levels deep get fully marked.
- [0256 — gc-capturing-closures](0256-gc-capturing-closures.md) — Third N2 sub-ADR. Lifts the chunk-safe predicate's capturing-closure rejection. Function-kind chunk slots register as roots; a new sibling propagation pass walks BLACK closure cells and marks their upvalue boxes BLACK. Bounded leak: upvalue box CONTENT marking (Strings / Tables held only via upvalues) deferred until per-upvalue type metadata lands.
- [0257 — gc-frame-root-walk](0257-gc-frame-root-walk.md) — Fourth N2 sub-ADR. Drops the ADR 0219 v1 depth-guard fallback for frame-safe chunks. Each user-fn entry pushes a 24-byte frame-root node carrying its String / Table / non-param Function slot ptrs into `g_frame_root_head`; the mark phase walks chunk roots + the frame chain, then runs the existing propagation passes. Mid-fn `collectgarbage()` now does real freeing. pcall save / restores the head alongside `g_gc_fn_depth`. TaggedValue locals in user fns still trigger the v1 fallback — N2-D-2 will add a TV frame chain.
- [0258 — close-runtime-dispatch](0258-close-runtime-dispatch.md) — First N3 sub-ADR. Lands the runtime `__close` metamethod call for `<close>` Locals on natural scope exit (Block + function-body fall-off). New `is_close` flag on `LocalInfo`; synthetic `HirStmtKind::CloseLocals { ids, candidates }` appended by both the `do … end` Block lowering and `lower_function_body`; codegen iterates in reverse, probes the metatable's `__close`, dispatches the Function via `emit_dispatch_chain_from_slot_ptr` with sig `(Number, Number) → ()` and placeholder `(0.0, 0.0)` args (matches user fn default param ABI). Nil / false-valued and non-Table-valued Locals skip silently per Lua 5.4 §3.3.8. Real `(Table, Nil)` arg pass-through, pcall / error-path close, return-path close, `goto`/`break` exit-edge close, and for/while/repeat body close all deferred to follow-ups.
- [0306 — int-divmod-r1c](0306-int-divmod-r1c.md) — F2-R1-c step 1. `//` and `%` join the integer-tree gate; `emit_int_expr` emits branch-free floor corrections (`-7 // 2 == -4`, `-7 % 2 == 1`) and `n//0` traps with "integer division by zero". Comparisons / fn bodies / boundaries / gate removal = remaining R1-c slices.
- [0305 — int-arith-r1b](0305-int-arith-r1b.md) — F2-R1-b. Gate widened to `+ - *` integer trees; `emit_int_expr` emits i64 arith with natural wraparound. Both ADR 0300 R1 probes fixed: `maxinteger + 1` wraps to mininteger, runtime >2^53 adds exact. FloorDiv/Mod/comparisons/boundaries = R1-c.
- [0304 — int-slots-r1a](0304-int-slots-r1a.md) — F2-R1-a implementation. `LocalInfo.int_slot` + `mark_integer_slot_eligible` (whole-chunk conservative gate) + i64 alloca/store/print codegen. `local big = 9007199254740993; print(big)` now exact. Arithmetic + fn bodies + boundaries = R1-b/c.
- [0303 — integer-slot-design](0303-integer-slot-design.md) — F2-R1 design. Subtype-keyed i64 slots (no new ValueKind): Number locals with `NumberSubtype::Integer` get i64 slots; conversions at dynamic boundaries. 132 consumer sites audited + grouped into R1-a (slots + read/write/print, eligibility-gated), R1-b (i64 BinOp + wraparound), R1-c (boundaries). 3-4 sessions.
- [0302 — mathtype-anyvalue](0302-mathtype-anyvalue.md) — F2-R3. `math.type` param kind → TaggedValue "any" sentinel; non-number args reach the classifier and yield nil per Lua §6.7 instead of a HIR arg-kind error. One-line gate change; codegen untouched.
- [0301 — mathtype-binop-subtype](0301-mathtype-binop-subtype.md) — F2-R2. ADR 0234's BinOp subtype classifier extracted to `classify_number_subtype`; `math.type` codegen now classifies BinOp args (`math.type(10 / 4)` → "float") instead of nil. Both consumers (propagation pass + math.type) share one fn. Call-result + UnaryOp subtypes deferred.
- [0300 — integer-subtype-audit](0300-integer-subtype-audit.md) — F2 status audit. Probe-verified: `math.type`, `//`, `1 == 1.0`, `tointeger`, bitwise, constants, i64 literals, static folds all already work (ADRs 0209-0214/0232). Residual re-scoped 6-10 → 2-3.5 sessions: R1 i64 runtime slots (wraparound + >2^53 locals), R2 subtype-through-BinOp (`math.type(10/4)`), R3 `math.type` any-value accept.
- [0299 — select-desugar](0299-select-desugar.md) — F1-D partial. `select("#", ...)` → `#_va_pack`; `select(n, ...)` → `_va_pack[n]` (single-value truncation per ADR 0228 precedent); `table.pack(...)` → pack alias + hoisted `.n` write. Pure HIR desugars in `lower_call` — no Builtin variants, zero plumbing ripple. Literal-args select + negative n + multi-value tail + `table.unpack` (dynamic-arity ABI) deferred.
- [0298 — vararg-spread-partial](0298-vararg-spread-partial.md) — F1-C-step3-partial. Pack forwarding (`inner(...)` passes the whole `_va_pack` by reference when `...` is the sole extra) + `{...}` lowers to a pack alias (enables `#{...}`; copy semantics deferred). Empty-pack `...` confirmed nil via the Index nil-on-missing path. `return ...` / `f(a, ...)` prefix spread blocked on a dynamic-arity ABI.
- [0297 — vararg-pack-step2](0297-vararg-pack-step2.md) — F1-C-step2 implementation of ADR 0296. Vararg fns get a synthetic `_va_pack` Table param; `ValueKind::Function(arity)` stores effective arity (declared + 1); call sites pack extras into a fresh Table; `...` in single-value position reads `_va_pack[1]`. `type(...)` now matches Lua with extras. Empty-pack nil + spread contexts remain step3.
- [0296 — vararg-pack-design](0296-vararg-pack-design.md) — F1-C-step2/3 design only. Pins Table-pack ABI (heap Table as pack, hidden last-position param on vararg fns, `Index(_va_pack, 1)` for single-value `...`) + spread contexts (`{...}`, `return ...`, `f(a, ...)`, multi-assign, `select`/`table.pack`/`table.unpack`). Session decomposition + zero-diff notes. No code changes.
- [0295 — vararg-codegen-stub](0295-vararg-codegen-stub.md) — F1-C-step1. Codegen for `HirExprKind::Vararg` emits `i1 0` (nil placeholder); `infer_kind` downgraded to `Nil`. HIR arity check at Ident-callee accepts extras when target is vararg (extras truncated). Real pack materialisation + spread deferred to step2/3; documented deviation table lists concrete behavior differences from Lua 5.4.
- [0294 — vararg-hir](0294-vararg-hir.md) — F1-B. HIR gains `HirFunction.is_vararg` + `HirExprKind::Vararg`. `LowerCtx.in_vararg_function` flag gates whether `...` is legal at each lowering site; non-vararg fn or chunk-level `...` errors. Codegen still errors with `UnsupportedExpr` — that removes in F1-C.
- [0293 — vararg-parser](0293-vararg-parser.md) — F1-A. Parser + AST gain `...`: new `TokenKind::DotDotDot`, `ExprKind::Vararg`, and `is_vararg: bool` on FunctionExpr / FunctionDef / MethodDef. Signature-position and expression-position both accepted; HIR errors loudly with `VarargUnsupported` pending F1-B/C wiring.
- [0292 — debug-traceback-stub](0292-debug-traceback-stub.md) — N7-21. Opens `debug.*` namespace and adds a `debug.traceback()` no-arg stub returning an empty String. Callers using `xpcall(f, debug.traceback)` now compile. Real stack walking + other `debug.*` deferred.
- [0291 — math-randomseed](0291-math-randomseed.md) — N7-20. `math.randomseed(seed)` via libc `srand`. New `srand` extern; codegen truncates the f64 seed to i32 and calls it. Void semantics (Print-placeholder Number result). Lua 5.4 no-arg secure form + 2-arg seeding deferred.
- [0290 — utf8-offset](0290-utf8-offset.md) — N7-19. `utf8.offset(s, n)` from-start form. Runtime scans bytes counting non-continuation boundaries; returns 1-indexed byte position of the n-th codepoint. Includes `n = codepoint_count + 1` end-boundary and `n = 0 → 1` convention. 3-arg form + negative-n backward walk deferred.
- [0289 — utf8-codepoint](0289-utf8-codepoint.md) — N7-18. `utf8.codepoint(s, i)` single-position form. Runtime decodes 1-4 UTF-8 bytes at `s[i-1..]`, returns i64 codepoint (-1 on error); codegen `sitofp`'s to Number. Range form + optional `i` default + invalid-UTF-8-as-Lua-error deferred.
- [0288 — utf8-char](0288-utf8-char.md) — N7-17. `utf8.char(n)` single-codepoint form. New runtime `lumelir_utf8_char_one(cp, out_buf)`; codegen reuses the ADR 0286 alloc + rewrite-length pattern. Invalid codepoints (< 0, > 0x10FFFF, surrogate range) return empty. Variadic form deferred.
- [0287 — pattern-balanced-frontier](0287-pattern-balanced-frontier.md) — N4-H. `%bxy` balanced-match via a depth counter; `%f[set]` zero-width frontier via `matches_set` on prev/current with `\0` at string edges. Both are special-cased at the top of `match_pattern` before the atom/quantifier flow.
- [0286 — string-gsub-string-repl](0286-string-gsub-string-repl.md) — N4-G. `string.gsub(s, pat, repl)` with String replacement. New runtime `lumelir_string_gsub_string_repl(..., out_buf, out_max)`; codegen upper-bounds `(s+1)*(r+1)+r+8`, allocates a String obj, fills payload via the bridge, then rewrites length header + NUL to the actual byte count. Also fixes anchor `^` semantics in `run_pattern_from` when `init_byte > 0`. Function/Table repl, `%N` backrefs, `n` limit, multi-return `(result, count)` deferred.
- [0285 — for-1name-parser](0285-for-1name-parser.md) — N4-F-2a. Parser accepts the 1-name generic-for `for x in iter_expr do ... end` by desugaring to the existing 2-name `ForGeneric` with a synthetic `_for1_discard` binding and `state, ctl = nil, nil`. HIR/codegen support for single-return iter and closure-returning gmatch source remain N4-F-2b/c.
- [0284 — string-gmatch-deferred](0284-string-gmatch-deferred.md) — N4-F-2 design only. Records the two structural blockers (builtin-returned closure synthesis + generic-for 1-name form) for the closure-iterator `string.gmatch`, plus a 3-sub-task decomposition (N4-F-2a parser/HIR for-in extension; N4-F-2b synthetic-function infrastructure; N4-F-2c the Builtin itself). No code changes.
- [0283 — string-find-init](0283-string-find-init.md) — N4-F-1 (gmatch foundation). `string.find` arity widened to (2, 3); new `lumelir_string_find_init(s, pat, init)` runtime entry built on a refactored `run_pattern_from`. Enables manual gmatch loops today; closure-returning `string.gmatch` is N4-F-2.
- [0282 — pattern-sets](0282-pattern-sets.md) — N4-E. Character sets `[abc]`, negated `[^abc]`, ranges `[a-z]`, classes inside `[%d%s]`. `atom_len` walks to the matching `]` (consuming `%X` so `%]` doesn't close); `matches_atom` dispatches `[` to a new `matches_set` that decodes literals/classes/ranges with leading-`^` negation. Composes with existing quantifiers, anchors, and the single capture.
- [0281 — pattern-single-capture](0281-pattern-single-capture.md) — N4-D. Adds single non-nested `(...)` capture support. Threads a `Copy` `CapState` through `match_pattern` / `match_max` / `match_opt` with snapshot-restore on every failed try. New runtime entry `lumelir_string_match_capture1` returns the first capture extent (or overall match when no capture present); `string.match` codegen switches to it. `string.find` keeps the overall-extent entry. Multi-capture multi-return, position capture, back-refs, captures-in-gmatch/gsub deferred.
- [0280 — pattern-anchors](0280-pattern-anchors.md) — N4-C. `^` (leading position only) and `$` (trailing position only) anchor the match; elsewhere they are literal. Outer search loop constrained to start=0 when leading `^` is detected; `match_pattern` terminates with `s_i == s.len()` check when the remaining pattern is exactly `$`.
- [0279 — pattern-quantifiers](0279-pattern-quantifiers.md) — N4-B. Greedy `*`/`+`/`?` on any single-byte atom. Pattern engine refactored to recursive backtracking (`match_pattern` + `match_max` + `match_opt`). `#![no_std]` bridge crate uses `unsafe { get_unchecked }` for all slice accesses (each call site bounds-checks first) to avoid `core::panicking::panic_bounds_check` link failure. Lazy `-`, sets, anchors, captures deferred.
- [0278 — string-match-pattern-engine-wire](0278-string-match-pattern-engine-wire.md) — N4-A. Switches `string.match` codegen from the literal-only `lumelir_string_find_plain` to the ADR 0231 pattern engine `lumelir_string_match_extents`, decoding the packed `(len, pos)` result and building a fresh String from the matched byte range. Patterns supported: literal, `.`, classes `%a %d %s %w %p` (+ siblings/complements). Quantifiers/anchors/captures/sets/`gmatch`/`gsub` deferred to N4-B–N4-G.
- [0277 — utf8-len](0277-utf8-len.md) — Sixteenth N7 sub-ADR. Opens `utf8.*` namespace with `utf8.len(s)`. Codegen: `scf.while` over the byte buffer counting non-continuation bytes (top 2 bits ≠ `0b10`). Range args + invalid-UTF-8 detection + other `utf8.*` deferred.
- [0276 — os-setlocale](0276-os-setlocale.md) — Fifteenth N7 sub-ADR. `os.setlocale()` no-arg form via libc `setlocale(LC_ALL, NULL)`; returns the current locale String. Setter form + cross-platform LC_ALL constant deferred.
- [0275 — math-modf](0275-math-modf.md) — Fourteenth N7 sub-ADR. `math.modf(x)` integer part — single-result position only via libm `trunc`. Multi-return `(int, frac)` deferred.
- [0274 — math-ult](0274-math-ult.md) — Thirteenth N7 sub-ADR. `math.ult(m, n)` unsigned 64-bit less-than. Codegen: `arith.fptosi` both Number args to i64, then `arith.cmpi ult` — single op, no libc dep. Number-subtype enforcement (Lua's "no integer representation" check) deferred.
- [0273 — math-log-base](0273-math-log-base.md) — Twelfth N7 sub-ADR. `math.log(x[, base])` arity widened to (1, 2) per Lua 5.4 §6.7. Codegen splits MathLog out of the unary-libm group; 1-arg dispatches libm `log`, 2-arg emits `log(x) / log(base)` via `arith.divf`.
- [0272 — os-execute](0272-os-execute.md) — Eleventh N7 sub-ADR. `os.execute(cmd)` via libc `system`; returns Bool = (rc == 0). Multi-return spec form deferred.
- [0271 — os-date](0271-os-date.md) — Tenth N7 sub-ADR. `os.date()` no-arg form via libc `ctime(&time(NULL))` with trailing newline stripped. Format string + `time` arg deferred until strftime plumbing.
- [0270 — math-atan-2arg](0270-math-atan-2arg.md) — Ninth N7 sub-ADR. `math.atan(y[, x])` arity widened to (1, 2) per Lua 5.4 §6.7 (replacing deprecated math.atan2). Codegen splits MathAtan out of the unary-libm group; 1-arg dispatches libm `atan`, 2-arg dispatches libm `atan2`.
- [0269 — os-difftime](0269-os-difftime.md) — Eighth N7 sub-ADR. `os.difftime(t2, t1)` via inline `arith.subf`. No libc dependency.
- [0268 — math-deg-rad](0268-math-deg-rad.md) — Seventh N7 sub-ADR. `math.deg(x)` and `math.rad(x)` via inline `arith.mulf` with compile-time-folded conversion factor (`180/π` and `π/180`). No libm dependency.
- [0267 — os-tmpname](0267-os-tmpname.md) — Sixth N7 sub-ADR. `os.tmpname()` via libc `tmpnam(NULL)` + strlen + `emit_string_obj_from_bytes` wrap. POSIX-deprecated tmpnam noted; mkstemp swap deferred.
- [0266 — os-rename](0266-os-rename.md) — Fifth N7 sub-ADR. `os.rename(old, new)` via libc `rename`. Returns Bool. `(nil, errmsg)` deferred.
- [0265 — os-remove](0265-os-remove.md) — Fourth N7 sub-ADR. `os.remove(filename)` via libc `remove(const char*)`. Returns Bool (true on success). `(nil, errmsg)` multi-return is a focused follow-up.
- [0264 — os-exit](0264-os-exit.md) — Third N7 sub-ADR. `os.exit([code])` via libc `exit(int)` with default code 0. Boolean code form and the `close` flag deferred.
- [0263 — math-random](0263-math-random.md) — Second N7 sub-ADR. `math.random([m[, n]])` via libc `rand()`: 0-arg → f64 in `[0, 1)` via `sitofp(rand()) / 2147483647.0`; 1-arg → `fmod(rand, n) + 1`; 2-arg → `m + fmod(rand, n - m + 1)`. `math.randomseed`, integer-flavor result, and higher-quality PRNG (Lua 5.4's xoshiro256\*\*) all deferred.
- [0262 — math-fmod](0262-math-fmod.md) — First N7 sub-ADR. `math.fmod(x, y)` via libm `fmod` — C-style truncation remainder, distinct from Lua's floor-mod `%` operator. Sig `(Number, Number) → Number`.
- [0261 — userdata-newproxy](0261-userdata-newproxy.md) — Fourth and final N3 sub-ADR. Lands the userdata type-system plumbing: `TAG_USERDATA = 7` tag, `GC_TYPE_USERDATA = 8` GC type, `s_typename_userdata` global, `emit_type_tagged_local` extended with a userdata arm, and a `Builtin::Newproxy` arm that allocates a 16-byte GC-tracked userdata payload and wraps it in a TaggedValue slot. `type(newproxy(false))` returns `"userdata"`. The full `ValueKind::Userdata` enum variant is avoided to dodge a ~50-site ripple; the TaggedValue path lands observable user behavior. Userdata equality (TAG_USERDATA eq arm), metamethod dispatch on userdata receivers, `setmetatable` on userdata, and spec-blessed `newuserdata(size)` are all separate follow-ups. **Closes the N3 milestone.**
- [0260 — mode-weak-clearing](0260-mode-weak-clearing.md) — Third N3 sub-ADR. Full weak-value semantics in two layers: (1) Pre-sweep weak-clear pass walks `g_gc_head` Tables, probes `mt["__mode"]`, for non-Nil mode scans the table's hash bucket entries and nulls value-slots whose `TAG_TABLE` payload references a WHITE GC node. (2) Mark-propagation `__mode` skip: inside the BLACK-Table propagation arm a `not_weak` flag is stashed in a 1-byte alloca and the inner mark calls (array-walk + hash-walk) gate on a load; weak-marked Tables don't propagate to their values so they reach sweep WHITE. Spec-precise `"k"`/`"v"`/`"kv"` discrimination, array-part weak clearing, weak-key clearing, and ephemerons all deferred to focused follow-ups.
- [0259 — gc-finalizer-dispatch](0259-gc-finalizer-dispatch.md) — Second N3 sub-ADR. Lifts the ADR 0238 `__gc` runtime-deferral pin. At sweep time, before unlink+free, each WHITE GC node has its type tag probed; for `GC_TYPE_TABLE` the payload's `metatable_ptr` is loaded, `mt["__gc"]` probed, and a `TAG_FUNCTION` value dispatched via `emit_dispatch_chain_from_slot_ptr` with sig `(Number) → ()` and placeholder arg `0.0` (same ABI strategy as ADR 0258). Single-pass — Lua spec's two-pass collect-then-fire-then-free for resurrection safety remains deferred per ADR 0163. Real `(Table)` arg pass-through and `__gc` on userdata are out of scope.

### Feature Implementation Memos

Historical records of Phase 2 feature increments. Useful for "how was X implemented" archaeology but not architectural reading.

<details>
<summary>91 entries — click to expand</summary>

- [0007 — phase2-0-local-and-multistmt](0007-phase2-0-local-and-multistmt.md)
- [0008 — phase2-1-assign-and-blocks](0008-phase2-1-assign-and-blocks.md)
- [0009 — phase2-2a-arith-operators](0009-phase2-2a-arith-operators.md)
- [0010 — phase2-2b-comparisons](0010-phase2-2b-comparisons.md)
- [0011 — phase2-3a-nil-and-perslot-types](0011-phase2-3a-nil-and-perslot-types.md)
- [0012 — phase2-3b-control-flow](0012-phase2-3b-control-flow.md)
- [0013 — phase2-3c-logical-operators](0013-phase2-3c-logical-operators.md)
- [0014 — phase2-3d-numeric-for](0014-phase2-3d-numeric-for.md)
- [0015 — phase2-4-break](0015-phase2-4-break.md)
- [0016 — phase2-5a-functions](0016-phase2-5a-functions.md)
- [0017 — phase2-5b-first-class-functions](0017-phase2-5b-first-class-functions.md)
- [0018 — phase2-5b2-function-args](0018-phase2-5b2-function-args.md)
- [0019 — phase2-5b3-function-return](0019-phase2-5b3-function-return.md)
- [0020 — phase2-5e-bool-nil-signatures](0020-phase2-5e-bool-nil-signatures.md)
- [0021 — phase2-5d-multi-return](0021-phase2-5d-multi-return.md)
- [0022 — phase2-2c-floor-and-bitwise](0022-phase2-2c-floor-and-bitwise.md)
- [0023 — phase2-2d-number-literals](0023-phase2-2d-number-literals.md)
- [0024 — phase2-7a-strings](0024-phase2-7a-strings.md) — (ABI surface superseded by ADR 0112)
- [0025 — phase2-7b-string-concat](0025-phase2-7b-string-concat.md)
- [0026 — phase2-7c-tostring](0026-phase2-7c-tostring.md)
- [0027 — phase2-7d-string-compare](0027-phase2-7d-string-compare.md)
- [0028 — phase2-7e-tonumber](0028-phase2-7e-tonumber.md)
- [0029 — phase2-7f-type](0029-phase2-7f-type.md)
- [0030 — phase2-7g-assert](0030-phase2-7g-assert.md)
- [0031 — phase2-8a-comments](0031-phase2-8a-comments.md)
- [0032 — phase2-8b-print-multi](0032-phase2-8b-print-multi.md)
- [0033 — phase2-7h-error](0033-phase2-7h-error.md)
- [0034 — phase2-8c-block-comments](0034-phase2-8c-block-comments.md)
- [0035 — phase2-4b-repeat-until](0035-phase2-4b-repeat-until.md)
- [0036 — phase2-5f-nested-functions](0036-phase2-5f-nested-functions.md)
- [0037 — phase2-5c-min-closures](0037-phase2-5c-min-closures.md)
- [0038 — phase2-7j-long-brackets](0038-phase2-7j-long-brackets.md)
- [0039 — phase2-7k-extended-escapes](0039-phase2-7k-extended-escapes.md)
- [0040 — phase2-7l-unicode-z-escapes](0040-phase2-7l-unicode-z-escapes.md)
- [0041 — phase2-8d-shebang](0041-phase2-8d-shebang.md)
- [0042 — phase2-5c1-top-level-capture](0042-phase2-5c1-top-level-capture.md)
- [0043 — phase2-5c2-kind-captures](0043-phase2-5c2-kind-captures.md)
- [0048 — phase2-0a-auto-declare-globals](0048-phase2-0a-auto-declare-globals.md)
- [0049 — phase2-1a-multi-target-assign](0049-phase2-1a-multi-target-assign.md)
- [0050 — phase2-1b-assign-multi-from-call](0050-phase2-1b-assign-multi-from-call.md)
- [0051 — phase2-7m-assert-with-message](0051-phase2-7m-assert-with-message.md)
- [0052 — phase2-7n-tostring-function](0052-phase2-7n-tostring-function.md)
- [0053 — phase2-6a-min-empty-tables](0053-phase2-6a-min-empty-tables.md)
- [0054 — phase2-6a-arr-array-index](0054-phase2-6a-arr-array-index.md)
- [0055 — phase2-6a-wr-array-write](0055-phase2-6a-wr-array-write.md)
- [0057 — phase2-6a-grow-array-push](0057-phase2-6a-grow-array-push.md)
- [0058 — phase2-6b-hash-keys](0058-phase2-6b-hash-keys.md)
- [0059 — phase2-6c-tag-arr](0059-phase2-6c-tag-arr.md)
- [0060 — phase2-6c-tag-hash](0060-phase2-6c-tag-hash.md)
- [0061 — phase2-6c-isnil-query](0061-phase2-6c-isnil-query.md)
- [0062 — phase2-6c-tag-hash-hard](0062-phase2-6c-tag-hash-hard.md)
- [0063 — phase2-6c-tag-locals](0063-phase2-6c-tag-locals.md)
- [0064 — phase2-6c-tag-hetero](0064-phase2-6c-tag-hetero.md)
- [0067 — phase2-6c-tag-consumers](0067-phase2-6c-tag-consumers.md)
- [0069 — phase2-6c-tag-defensive-trap](0069-phase2-6c-tag-defensive-trap.md)
- [0070 — phase2-6c-tag-consumers-inline](0070-phase2-6c-tag-consumers-inline.md)
- [0071 — phase2-6c-tag-fn-tbl](0071-phase2-6c-tag-fn-tbl.md)
- [0072 — phase2-6c-tag-fn-tbl-call](0072-phase2-6c-tag-fn-tbl-call.md)
- [0074 — phase2-6c-tag-locals-fn](0074-phase2-6c-tag-locals-fn.md)
- [0075 — phase2-6c-tag-callee-arity](0075-phase2-6c-tag-callee-arity.md)
- [0076 — phase2-6c-tag-locals-fn-multi](0076-phase2-6c-tag-locals-fn-multi.md)
- [0077 — phase2-7p-arith-string-coerce](0077-phase2-7p-arith-string-coerce.md)
- [0078 — phase2-8e-iter-ipairs](0078-phase2-8e-iter-ipairs.md)
- [0079 — phase2-6b-hash-keys](0079-phase2-6b-hash-keys.md) — tagged-key Plan E
- [0080 — phase2-8e-iter-pairs](0080-phase2-8e-iter-pairs.md)
- [0081 — phase2-8e-iter-next](0081-phase2-8e-iter-next.md)
- [0084 — phase2-8e-iter-tk](0084-phase2-8e-iter-tk.md)
- [0085 — phase2-8e-iter-generic](0085-phase2-8e-iter-generic.md)
- [0086 — phase2-6b-hash-key-nan](0086-phase2-6b-hash-key-nan.md)
- [0088 — phase2-6b-hash-lookup-miss](0088-phase2-6b-hash-lookup-miss.md)
- [0089 — phase2-7p-tagged-arith-coerce](0089-phase2-7p-tagged-arith-coerce.md)
- [0095 — phase2-nested-index-assign-widen](0095-phase2-nested-index-assign-widen.md)
- [0101 — phase2-stdlib-math](0101-phase2-stdlib-math.md)
- [0102 — phase2-stdlib-math-pow-trig-log](0102-phase2-stdlib-math-pow-trig-log.md)
- [0103 — phase2-stdlib-string-begin](0103-phase2-stdlib-string-begin.md)
- [0104 — phase2-stdlib-string-sub](0104-phase2-stdlib-string-sub.md)
- [0105 — phase2-stdlib-string-rep](0105-phase2-stdlib-string-rep.md)
- [0106 — phase2-stdlib-table-begin](0106-phase2-stdlib-table-begin.md)
- [0107 — phase2-stdlib-table-concat-sep](0107-phase2-stdlib-table-concat-sep.md)
- [0108 — phase2-stdlib-table-concat-bounds](0108-phase2-stdlib-table-concat-bounds.md)
- [0109 — phase2-stdlib-string-byte](0109-phase2-stdlib-string-byte.md)
- [0110 — phase2-stdlib-arg-kind-validation](0110-phase2-stdlib-arg-kind-validation.md)
- [0111 — phase2-stdlib-table-insert](0111-phase2-stdlib-table-insert.md)
- [0112 — phase2-string-abi-refactor](0112-phase2-string-abi-refactor.md) — supersedes 0024 ABI
- [0113 — phase2-string-char](0113-phase2-string-char.md)
- [0114 — phase2-emit-f2i-gate-sweep](0114-phase2-emit-f2i-gate-sweep.md) — cross-cutting Tidy First
- [0115 — phase2-run-arg-table](0115-phase2-run-arg-table.md)
- [0116 — phase2-stdlib-io-write](0116-phase2-stdlib-io-write.md)
- [0117 — phase2-stdout-fwrite](0117-phase2-stdout-fwrite.md) — resolves ADR 0112 NUL truncation
- [0118 — phase2-stdlib-table-remove](0118-phase2-stdlib-table-remove.md)
- [0119 — phase2-stdlib-io-read](0119-phase2-stdlib-io-read.md)

</details>

### Refactor Memos

Pure structural / tidying records.

- [0044 — phase2-5c3-closure-escapes](0044-phase2-5c3-closure-escapes.md) — superseded by ADR 0083
- [0045 — phase2-9a-diagnostics](0045-phase2-9a-diagnostics.md)
- [0046 — phase2-9b-source-snippets](0046-phase2-9b-source-snippets.md)
- [0047 — phase2-9c-strip-offset-from-display](0047-phase2-9c-strip-offset-from-display.md)
- [0056 — phase2-6a-norm-stable-table-header](0056-phase2-6a-norm-stable-table-header.md)
- [0065 — phase2-6c-tag-hetero-fix](0065-phase2-6c-tag-hetero-fix.md)
- [0066 — phase2-6c-tag-hetero-eq](0066-phase2-6c-tag-hetero-eq.md)
- [0073 — phase2-6c-tag-rs-split](0073-phase2-6c-tag-rs-split.md)
- [0087 — phase2-6b-hash-key-validity](0087-phase2-6b-hash-key-validity.md)
- [0090 — phase2-devinfra-emit](0090-phase2-devinfra-emit.md) — first `2.devinfra-*` lane
