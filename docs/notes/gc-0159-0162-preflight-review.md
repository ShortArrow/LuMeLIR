# GC ADRs 0159-0162 — Pre-Implementation Review (Codex 6 視点)

- **Date:** 2026-06-13
- **Reviewer:** ShortArrow
- **Scope:** ADRs 0159 (mark) / 0160 (stack walk) / 0161 (sweep) / 0162 (auto-trigger), cross-checked against the landed 0157 (allocator wrapper) and 0158 (migrate remaining types).
- **Purpose:** Codex 6 視点 (CLAUDE.md 第1-第6原則) pre-flight check before implementation of ADR 0159 begins. Findings here feed forward into the implementation ADR (provisionally **ADR 0184 = GC mark phase implementation**) and any preparatory Tidy First ADR.

## Verdict

**GREEN LIGHT for implementation**, conditional on three robustness fixes (R1 / R2 / R3 below) being absorbed into the implementation ADR or a preparatory Tidy First ADR. Cross-ADR consistency is solid; header layouts, per-type references, and threshold globals all align between 0156 / 0157 / 0158 / 0159 / 0161 / 0162.

## Findings by axis

### 第1原則 — spec / requirement adherence

- ✅ Lua 5.4 GC spec conformance scope matches ADR 0145 (non-moving mark-and-sweep, Phase 2 = leak / Phase 3 = collected).
- ✅ Documented gaps (`__gc` finalizers → 0163, weak tables → 0164) are explicit, not silent omissions.
- ⚠ Safety mode in ADR 0159:62 (pre-mark BLACK before stack walk lands) costs an extra O(n) heap walk per collection during v1. Not a spec gap but worth budgeting in the auto-trigger threshold logic (ADR 0162) so the first collection doesn't appear to stall.

### 第2原則 — TDD

- ✅ Existing `tests/phase3_gc_*.rs` cover allocator-count observability (3 e2e via `collectgarbage("count")`).
- ⚠ No mark / sweep / threshold-boundary tests exist yet. Implementation ADR must add Red Day 0 e2e covering at minimum:
  - Zero-root mark (empty roots, all WHITE after pre-scan disabled).
  - Sweep idempotence (safety-mode pre-BLACK → sweep frees nothing).
  - Threshold boundary (1 MiB − 1 / 1 MiB / 1 MiB + 1 byte allocations).
  - Doubling logic when `post_sweep × 2 > threshold`.
- ⚠ No edge-case tests for zero-length tables / nil-only array slots / hash-only tables. Required because mark phase's per-type walks branch on `length > 0` and `cap > 0`.

### 第3原則 — anti-ad-hoc / Tidy First / Robust

Three concrete robustness gaps:

#### R1 — Worklist capacity strategy

**Location:** ADR 0159:55 — "alloca-based 4096-capacity worklist; bumped in follow-up if needed."

**Risk:** The "bumped in follow-up" wording is an ad-hoc placeholder. A real metatable chain with deep `__index` Table-form hops (ADR 0167 multi-hop dispatch) can saturate at runtime; alloca overflow has no graceful path.

**Options:**
1. Grow dynamically via `malloc` (heap-based stack); decide whether the worklist itself is GC-tracked or a scratch allocation.
2. Hard-cap at 4096 and document as a Phase 3 v1 constraint with a known-failure test.
3. Pre-size to `(g_gc_total_bytes / GC_HEADER_SIZE)` upper bound at collection start.

**Recommendation:** Option 1 (heap-based, allocated outside `emit_gc_alloc` so not tracked) — minimum complexity, no false ceiling.

#### R2 — Per-type dispatch chokepoint missing

**Location:** ADR 0159:51-58 implies a per-type switch keyed on `GC_HEADER_OFF_TYPE_TAG`. Existing `tagged.rs` already has `policy_for_tag` for `TAG_*` value dispatch but no equivalent for `GC_TYPE_*` allocator dispatch.

**Risk:** Mark phase (ADR 0159), sweep phase (ADR 0161 finalizer hook in 0163), and potentially future debug / introspection paths will each open-code the same `match type_tag { … }`. Drift between them is silent. The CLAUDE.md 第3原則 (anti-ad-hoc) prescribes consolidating at the design boundary.

**Options:**
1. Extract `gc_type_references(type_tag) -> &'static [GcReferenceField]` decision table in `tagged.rs`. Mark phase iterates it; future passes too.
2. Leave open-coded for v1, refactor when 2nd consumer arrives.

**Recommendation:** Option 1 — Tidy First before the duplication exists; precedent in ADR 0182 (`mark_ident_as` consolidation done with only 2 consumers).

#### R3 — `GC_HEADER_OFF_SIZE` u32 truncation guard

**Location:** ADR 0157:78 — `*(raw + 4) = trunc(payload_size, u32)`. The size field is u32; payloads ≥ 4 GiB silently wrap.

**Risk:** ADR 0112 string-object ABI permits arbitrary string lengths. A user could theoretically allocate a > 4 GiB string (e.g. `string.rep("x", 5_000_000_000)`); the truncation makes the sweep free-size wrong (frees 4 GiB minus wrap, leaks the rest, or worse, mismatches `g_gc_total_bytes` accounting).

**Options:**
1. Widen field to u64 (header grows from 16 → 24 bytes; affects every existing offset). Disruptive but principled.
2. Assert `payload_size < (1 << 32)` in `emit_gc_alloc`, trap on violation. Cheap; punts the issue.
3. Document as a 4 GiB per-object cap; add static check.

**Recommendation:** Option 2 — minimum disruption, fail-fast. Option 1 should be revisited when the first > 4 GiB string is a real user requirement.

### 第4原則 — clean architecture

- ✅ Dependency direction is acyclic: `tagged.rs` (constants) ← `primitive.rs` (`emit_gc_alloc`) ← `emit.rs` (mark/sweep dispatch).
- ✅ No `gc.rs` module exists yet; mark/sweep will live in `emit.rs`. Acceptable for v1; promote to its own module if surface grows past ~400 LOC.
- ✅ Per-type layout offsets (TABLE_OFF_*, GC_HEADER_OFF_*) defined once in `tagged.rs`, consumed downstream.

### 第5原則 — Given/When/Then state machine

- ✅ WHITE/GREY/BLACK state transitions are crisp for each axis (mark / safety mode / sweep / threshold).
- ⚠ Mid-collection interruption (ADR 0160:57 documents "safe only when no MLIR code executing other than collectgarbage()") leaves undefined state if a future coroutine ADR pre-empts. Reserve as a known limitation; ADR 0165+ (coroutines) inherits the constraint.

### 第6原則 — intent via naming / structure / documentation

- ✅ All planned function names (`emit_gc_mark`, `emit_gc_sweep`, `emit_gc_alloc`) are explicit.
- ✅ Decision tables in 0159:42-49 (per-type reference walks) and 0162:32-39 (threshold doubling) read clearly.
- ⚠ Minor docstring gaps: 0159 doesn't explicitly state that v1 returns `collected=0` from `collectgarbage()`; 0162 doesn't note overflow behaviour at the 1 GiB doubling cap. Low-impact, fix during implementation.

## Cross-ADR consistency

| Pair | Concern | Status |
|---|---|---|
| 0157 ↔ 0159 | Header layout (mark @ 0, type_tag @ 1, size @ 4, next @ 8, payload @ 16) | ✅ verified against `src/codegen/primitive.rs` |
| 0158 ↔ 0159 | All 8 migrated sites populate the header that mark phase walks | ✅ verified — every `emit_gc_alloc` callsite tagged correctly |
| 0159 ↔ 0161 | Per-type reference list (mark) vs free order (sweep) | ✅ no conflict — sweep frees via `free()` without revisiting per-type walks |
| 0156 ↔ 0162 | Threshold global vs per-object header — no overlap | ✅ separate concerns |
| 0159 ↔ 0160 | Stack-walk slot table interface | ⚠ designed but unimplemented; coupling will be exercised at first implementation |

## Action items for the implementation ADR (ADR 0184 candidate)

1. **R1** (worklist) — pick Option 1 (heap-based grow) and document the chosen capacity strategy in the ADR Scope (literal).
2. **R2** (per-type dispatch table) — split into a small Tidy First ADR (e.g. **ADR 0184 = `gc_type_references` decision table in tagged.rs`**, behaviour-preserving since no mark phase exists yet — it pre-stages the consumer). Mark phase implementation lands as **ADR 0185**.
3. **R3** (size truncation) — add a single `arith.cmpi(ult, payload_size, 1<<32)` + trap branch inside `emit_gc_alloc` in the same commit as R1 or before.
4. **TDD gap** — Red Day 0 e2e suite for mark + sweep + threshold boundary lands as part of the implementation ADR's C2.
5. **Coroutine-safety hazard** — documented in this memo; deferred to coroutine ADR.

## References

- [ADR 0156](../design/0156-gc-architecture-v1.md) — parent architecture.
- [ADR 0157](../design/0157-gc-allocator-wrapper.md) — landed allocator wrapper.
- [ADR 0158](../design/0158-gc-migrate-remaining-types.md) — landed migration.
- [ADR 0159](../design/0159-gc-mark-phase.md) — mark phase design.
- [ADR 0160](../design/0160-gc-stack-walk.md) — stack walk design.
- [ADR 0161](../design/0161-gc-sweep-phase.md) — sweep design.
- [ADR 0162](../design/0162-gc-auto-trigger.md) — auto-trigger threshold.
- [ADR 0145](../design/0145-gc-strategy.md) — Phase 2 = leak, Phase 3 = mark-and-sweep decision.
- [ADR 0182](../design/0182-param-inference-kind-parameterised-helpers.md) — Tidy First-before-third-consumer precedent (cited in R2 recommendation).
