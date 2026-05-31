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
