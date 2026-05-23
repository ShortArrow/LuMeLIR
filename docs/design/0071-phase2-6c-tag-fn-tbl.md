# 0071. Phase 2.6c-tag-fn-tbl: Function and Table Values in Tagged Slots

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-05
- **Deciders:** ShortArrow

## Context

ADR 0064 reserved `TAG_FUNCTION = 4` / `TAG_TABLE = 5` for a
follow-up sub-phase but left HIR rejecting Function / Table
values in tables. That decision rolled into one pure pending
LIC: `LIC-2.6c-tag-hetero-fn-tbl-1`. ADR 0069's defensive trap
(`emit_tagged_unknown_tag_trap`) made it safe to flip the
toggle — any tag the consumers don't yet handle now fail-fast
instead of silently misbehaving.

Codex review (post-ADR-0070) put this LIC at the top of the
queue: "唯一の pure pending、ADR 0069 の trap が fail-fast
guard rail として機能 → 安全に進められる。" The same review
also flagged the `Index → tmp tagged slot → consumer` pattern
(Print / Type / ToString) as rule-of-three triggered for a
small Tidy First.

This ADR delivers both: a Tidy First commit extracting
`emit_inline_index_into_tagged_tmp`, then a feature commit
opening Function (closure-less) and Table values into tagged
slots and extending the four consumer dispatch helpers.

User-visible:
```lua
local function f() return 42 end
local t = {f, "hi", {1, 2}}
print(type(t[1]))           -- "function"
print(type(t[2]))           -- "string"
print(type(t[3]))           -- "table"
print(tostring(t[1]))       -- "function"

local u = {}
local v = {}
local w = {u, u, v}
local a = w[1]
local b = w[2]
local c = w[3]
print(a == b)               -- true (reference equality)
print(a == c)               -- false
```

## Decision

### Phase A (Tidy First): `emit_inline_index_into_tagged_tmp`

Print (ADR 0065), Type, and ToString (ADR 0067 / 0070) repeated
the same five-line shape: alloca a `TaggedValue` slot, fill it
via `emit_local_init_tagged`, return the slot for the consumer
to dispatch on. Three call sites = rule of three.

```rust
fn emit_inline_index_into_tagged_tmp(
    target: &HirExpr, key: &HirExpr, …,
) -> Result<Value, CodegenError> {
    let tmp = emit_alloca_slot_for_kind(TaggedValue, …);
    emit_local_init_tagged(tmp, target, key, …)?;
    Ok(tmp)
}
```

Each call site collapses from ~13 lines to one helper call plus
the consumer dispatch. Behaviour-preserving; 836 tests still
green after the Tidy First commit.

### Phase B (Feature): TAG_FUNCTION / TAG_TABLE

#### Tag space

```rust
const TAG_FUNCTION: i64 = 4;
const TAG_TABLE: i64 = 5;
```

ADR 0064's reserved range (2..=5) is now fully assigned.

#### Payload

| Tag         | Payload type | Notes |
|-------------|--------------|-------|
| TAG_FUNCTION | `!llvm.ptr` | Function value bridged to `ptr` via `emit_unrealized_cast` (ADR 0019) before the 8-byte store. |
| TAG_TABLE    | `!llvm.ptr` | Table value is already the stable header pointer (ADR 0056) — store directly. |

8-byte each, so the existing raw-i64 slot copy (ADR 0064)
round-trips without bitcasts.

#### Two new store helpers + dispatcher arm

```rust
fn emit_value_slot_store_function(slot, value) {
    // store TAG_FUNCTION; ucast(value, ptr); store at +8
}
fn emit_value_slot_store_table(slot, value) {
    // store TAG_TABLE; store value (ptr) at +8
}
```

`emit_value_slot_store_dispatched` gains the `Function(_)` and
`Table` arms; `Table` constructor / array-key IndexAssign / hash
IndexAssign all share that single dispatcher.

#### HIR `value_ok` matrix

`HirStmtKind::IndexAssign` and `HirExprKind::Table` both extend
their accept set:

```rust
// IndexAssign:
let value_ok = matches!(
    (key_kind, value_kind),
    (Number, Number) | (Number, Bool) | (Number, String)
        | (Number, Function(_)) | (Number, Table)
        | (String, Number) | (String, Bool) | (String, String) | (String, Nil)
        | (String, Function(_)) | (String, Table)
);

// Table constructor: Number / Bool / String / Nil / Function(_) / Table.
```

#### Closure-with-upvalues escape rejection

The same ADR 0044 `HirError::ClosureEscapes` analysis that
already rejects closure escapes via call args / return values
now extends to table storage. A new helper `function_ref_id`
unifies "is this expression a function reference?" across
`HirExprKind::FunctionRef(fid)` and
`HirExprKind::Local(LocalId)` with `info.func_id`. When the
referenced function has non-empty `upvalues`, `IndexAssign` and
`Table` constructor reject with a `position: "table value"` /
`"table element"` annotation.

This keeps closures-with-upvalues firmly inside the
already-tested `2.5c-min` direct-call sandbox and tracks the
relaxation as `LIC-2.6c-tag-hetero-closure-escape-1`. Plain
`function() ... end` literals (no captures) and top-level
`local function f` references pass through.

#### Consumer dispatch extension (4 helpers)

`emit_print_tagged_local`, `emit_type_tagged_local`,
`emit_tostring_tagged_local`, and `emit_tagged_eq_local_local`
each grew nested `scf.if` arms in their previously-trapping
`else` branches:

| Helper | TAG_FUNCTION arm | TAG_TABLE arm | Truly-unknown arm |
|--------|------------------|---------------|-------------------|
| print  | `printf("%s", "function")` | `printf("%s", "table")` | `emit_tagged_unknown_tag_trap` (ADR 0069) |
| type   | yield `s_typename_function` | yield `s_typename_table` | trap |
| tostring | yield `s_typename_function` | yield `s_typename_table` | trap |
| eq     | `ptrtoint(lhs_payload) == ptrtoint(rhs_payload)` | same | trap |

Function / Table `==` is **reference equality** per Lua spec —
ptrtoint both sides and compare. Address-prefixed prints
(`"function: 0x..."`) are out of scope; the literal typename is
sufficient for current matrix tests.

#### `Local(TaggedValue)` read stays Number-only

The `HirExprKind::Local` arm in `emit_expr` continues to call
`emit_value_slot_check_number` and extract `f64`. Function and
Table tagged values are observable through the four consumer
helpers but **not callable / dereferenceable through a
widened local**. Logged as
`LIC-2.6c-tag-hetero-fn-tbl-call-1`; the relaxation needs a
tag-aware Local read (and a follow-up function-call codegen).

### Out-of-scope consumers

`assert`, `error`, `tonumber`, arithmetic, ordering — all keep
their current paths. Lua-spec-correct (arith on
function/table errors) or cosmetic (assert/error operate on
truthiness, mostly orthogonal).

## TDD Process

1. **Step 1 — Red.** 15 e2e tests in
   `tests/phase2_6c_tag_hetero_fn_tbl.rs`: 13 fail, 2 pass
   (the closure-escape rejection — already enforced by the
   existing `ClosureEscapes` analysis on the function-ref
   shape — and the regression backstop).
2. **Step 2a — HIR Green.** `value_ok` matrix expansion +
   `function_ref_id` helper + closure-escape gate. 4 tests
   advance.
3. **Step 2b — Codegen TAG / store / dispatch Green.** Tag
   constants, two new `_store_*` helpers + dispatcher arm,
   four consumer dispatch chains extended. All 15 fn/tbl
   tests pass at 851 (= 836 + 15) total.
4. **Step 3 — Negative-test reframe.** Three pre-ADR-0064 /
   ADR-0069 tests (`function_element_is_static_error_post_2_6c
   _hetero` and friends) used to assert plain Function values
   reject; reframe each to assert `closure_with_upvalue_*`
   rejects instead. The defensive-trap negative tests gain a
   positive foil (`closure_less_function_in_array_now_accepted
   _post_2_6c_tag_fn_tbl`).
5. **Step 4 — Documentation + ADR + commit.**

## Alternatives Considered

- **Allow closures with upvalues in tables.** Would close the
  symmetric gap with ADR 0044 (call args / returns) but
  requires extending escape analysis to table reads (because
  the table can outlive the upvalue's stack frame). Out of
  scope; tracked as `LIC-2.6c-tag-hetero-closure-escape-1`.
- **Address-prefixed `print(function)` / `print(table)`.** Lua
  prints `"function: 0x123"` / `"table: 0x123"`. Requires
  `snprintf("%p")` paths plus a stable address that survives
  GC (we have no GC). The literal typename is honest and
  visually similar.
- **Make `Local(TaggedValue)` Function-callable in this phase.**
  Touches the call codegen (`Callee::Indirect` extended),
  function-pointer ucast at the extract site, and possibly
  `MultiAssignFromCall`. Defers cleanly to a follow-up
  (`LIC-2.6c-tag-hetero-fn-tbl-call-1`); the four consumer
  helpers cover the read-side surface that users hit first.
- **`tagged.rs` module split as part of this phase.** Codex
  has flagged this for four phases now. Each time, the cross-
  module visibility design is the gating concern. Defer again;
  ADR 0071 ships the rule-of-three Tidy First as a smaller
  step in the right direction.

## Consequences

- ~480 LOC in `src/codegen/emit.rs` (TAG constants + 2 store
  helpers + dispatcher arm + 4 consumer dispatch chains
  extended), plus a ~25-line Tidy First helper extraction in a
  separate commit.
- ~40 LOC in `src/hir/mod.rs` (value_ok / elem_ok matrix,
  `function_ref_id` helper, two `ClosureEscapes` annotation
  sites).
- 15 new e2e tests in
  `tests/phase2_6c_tag_hetero_fn_tbl.rs`. Total green at
  **851** (= 836 + 15). Six pre-existing reject tests reframe
  for the new contract (closure-with-upvalues stays rejected).
- **`LIC-2.6c-tag-hetero-fn-tbl-1` → resolved.** With it,
  `LIC-2.6a-arr-2`, `LIC-2.6a-wr-3`, `LIC-2.6b-hash-2` flip
  partial → resolved (all six tag kinds now supported as table
  values).
- New LIC entries:
  - `LIC-2.6c-tag-hetero-closure-escape-1` — closures with
    upvalues stored in tables (HIR-rejected today).
  - `LIC-2.6c-tag-hetero-fn-tbl-call-1` — calling a Function
    value retrieved through a tagged slot (Local read still
    f64-only).
- The `emit_tagged_unknown_tag_trap` site introduced in ADR
  0069 is now genuinely the "future tag value" guard rail
  (currently unreachable because HIR caps at tag 5).

## Documentation updates

- [x] §1 slot layout — TAG_FUNCTION / TAG_TABLE rows added,
      constant block reproduces ADR 0071's tag definitions.
- [x] §2 producer / source taxonomy — IndexAssign / Table
      rows updated to enumerate all six accepted kinds; new
      `closure with upvalues` row marks the open LIC.
- [x] §3 consumer coverage matrix — Function / Table columns
      added across `print`, `type`, `tostring`, and `==`
      Local-Local rows. Trap commentary updated to "truly-
      unknown tag (≥ 6)".
- [x] §4 LIC consolidation — `fn-tbl-1` and the three
      partial entries promoted to Resolved; two new pending
      entries (`closure-escape-1`, `fn-tbl-call-1`) added.
      Totals: 12 resolved / 1 partial / 2 pending.
- [x] §5 runtime tag invariants — n/a (invariants unchanged).
- [x] §7 open questions — re-prioritised; closure-escape and
      fn-tbl-call moved up; tagged.rs split deferred again.
- [x] §8 ADR index — ADR 0071 row added; "Last updated"
      bumped to "after ADR 0071".

## Lua-Incompatibility Tracker

See `docs/design/tagged-semantics.md` §4 for the authoritative
list (ADR 0068).
