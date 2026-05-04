# 0064. Phase 2.6c-tag-hetero: Heterogeneous Bool / String Table Values

- **Status:** Accepted
- **Date:** 2026-05-04
- **Deciders:** ShortArrow

## Context

ADR 0063 widened `local x = t[i]` into a 16-byte tagged slot
(`MaybeNilNumber`) so locals could carry Number-or-Nil. The next
LIC entries on the table-side were:

- `LIC-2.6a-arr-2` / `LIC-2.6a-wr-3`: array values are
  Number-only.
- `LIC-2.6b-hash-2`: hash values are Number-only (Nil delete is
  partial).

Lua treats tables as heterogeneous: `{1, "hello", true}` and
`t.k = some_string` are everyday code. This ADR makes Bool /
String values land in the same 16-byte tagged slot already used
by tagged-arr (ADR 0059), tagged-hash (ADR 0060), and tagged-
locals (ADR 0063). Function and Table values stay out of scope —
they need ucast / cycle / closure-escape handling that warrants
a separate sub-phase.

## Decision

### `MaybeNilNumber` → `TaggedValue` (Tidy First)

A separate refactor commit renames the locals-widening kind so
the name reflects what the slot actually carries after this
phase. Behaviour-preserving: the tag still defaults to Number-
or-Nil and Local read still extracts f64 with trap-on-non-Number.

### Tag space extension

```rust
const TAG_NIL: i64 = 0;     // existing
const TAG_NUMBER: i64 = 1;  // existing
const TAG_BOOL: i64 = 2;    // new
const TAG_STRING: i64 = 3;  // new
// TAG_FUNCTION = 4, TAG_TABLE = 5: reserved for a follow-up.
```

### Slot payload layout

The 16-byte slot is unchanged in layout — `{i64 tag, 8-byte
payload}` — only the payload type now varies with the tag:

| Tag         | Payload type            |
|-------------|-------------------------|
| TAG_NIL     | `0` (unused)            |
| TAG_NUMBER  | `f64`                   |
| TAG_BOOL    | `i1` zero-extended to `i64` |
| TAG_STRING  | `!llvm.ptr` (8 bytes)   |

Internal copies between slots load the payload as `i64` so the
8-byte field round-trips without caring about the runtime type.

### Write-side changes

#### HIR `value_ok` matrix

`HirStmtKind::IndexAssign` accepts:
- `(Number key, Number / Bool / String value)` — array writes
- `(String key, Number / Bool / String / Nil value)` — hash
  writes (Nil retains its hard-tombstone meaning from ADR 0062)

#### Table constructor

`HirExprKind::Table` allows Number / Bool / String / Nil
elements. Function and Table elements still reject (closure-
escape / ucast).

#### Codegen store helpers

Three sibling helpers emit `{tag, payload}` for the shared
16-byte slot layout:

- `emit_value_slot_store_number(slot, f64)` — existing
- `emit_value_slot_store_nil(slot)` — existing
- `emit_value_slot_store_bool(slot, i1)` — new (zero-extends to
  i64 before the store)
- `emit_value_slot_store_string(slot, ptr)` — new

A new dispatcher `emit_value_slot_store_dispatched(slot, value,
kind)` routes to the matching helper. Table constructor,
IndexAssign Number-key, and IndexAssign String-key Number/Bool/
String paths all call through it. The Nil hash path (hard
delete, ADR 0062) is left as a dedicated arm because it also
overwrites the key with `HASH_DELETED_KEY`.

### Read-side changes

The hardest piece. Local(TaggedValue) cannot eagerly extract a
single static type, since the runtime tag determines the
payload's static type. We pick **context dispatch**: the
consumer site special-cases TaggedValue and reads the slot
directly with the appropriate runtime branch.

#### `print(Local(TaggedValue))`

The `Builtin::Print` arm of `emit_expr` checks each argument:
when the argument is `HirExprKind::Local(idx)` and `idx`'s kind
is `TaggedValue`, it bypasses `emit_expr` (which would force the
trapping Number-only path) and calls
`emit_print_tagged_local(slot_ptr)`. That helper builds a chain
of nested `scf.if` over the tag:

```text
if tag == TAG_NUMBER:
    printf("%g", load_f64(slot+8))
else if tag == TAG_BOOL:
    printf("%s", select(load_i1(slot+8), s_true, s_false))
else if tag == TAG_STRING:
    printf("%s", load_ptr(slot+8))
else:
    printf("%s", s_nil)
```

#### Arithmetic / comparison on Local(TaggedValue)

Extract path is unchanged from ADR 0063: tag check Number, trap
otherwise, load f64 at offset +8. This matches Lua's "nil + 1
errors" / "string + number errors when not coercible" semantics
at the sub-phase level. String coercion to number on arith is
out of scope.

#### `if x == nil` for Local(TaggedValue)

`HirExprKind::IsNilLocal` (ADR 0063) is unchanged — it reads the
tag at slot+0 and compares with `TAG_NIL`. Bool / String / Number
all answer false; Nil answers true.

### CA invariants preserved

| Layer    | Change                                                                  |
|----------|-------------------------------------------------------------------------|
| Lexer    | None                                                                    |
| Parser   | None                                                                    |
| AST      | None                                                                    |
| HIR      | `value_ok` matrix expansion, Table constructor element kind expansion. The TaggedValue rename happens in a separate Tidy First commit. |
| Codegen  | Two new tag constants, two new store helpers + dispatcher, payload copies via i64, `emit_print_tagged_local`, `print` arg arm dispatches on Local(TaggedValue). |

## TDD Process

1. **Step 0 — Tidy First.** Mechanical
   `MaybeNilNumber` → `TaggedValue` rename. 769 tests stay
   green. Separate commit.
2. **Step 1 — Red.** 12 e2e tests in
   `tests/phase2_6c_tag_hetero.rs`. 10 fail (heterogeneous
   write/read), 2 pass (regression: existing Number-only
   reads).
3. **Step 2 — Green.** Tag constants + store helpers, HIR
   `value_ok` matrix, Table constructor extension, IndexAssign
   value dispatch, payload copies generalised to i64, print
   tag dispatch. 4 invalidated existing tests are reframed
   (`non_number_element_is_static_error` etc) to reflect the
   new contract: Function values still reject, Bool / String
   accept. All 12 new tests pass at 781 total (= 769 + 12).
4. **Step 3 — ADR + AGENTS + commit.** Single feature commit.

## Alternatives Considered

- **Function / Table values too.** Function values need `ucast`
  bridging to / from `!llvm.ptr` and the closure-escape
  analysis (ADR 0044) needs to extend across table boundaries.
  Table values introduce alias / cycle considerations. Both
  warrant a focused sub-phase. Rejected for this round.
- **Separate ValueKind variant per inner kind** (`MaybeNilBool`,
  `MaybeNilString`). Avoids the runtime tag dispatch but
  multiplies the kind-pattern matches across HIR / codegen and
  loses the actual Lua semantic (a single local can hold any
  tag). Rejected.
- **String coercion on arith** (`"5" + 1` → 6 per Lua). Adds a
  runtime conversion path on every binop and is mostly
  orthogonal to the heterogeneity goal. Defer.
- **Read-side bypass `emit_expr` everywhere.** The current
  approach only special-cases `print(Local(TaggedValue))`. A
  more general dispatch (every BinOp / every builtin) would
  raise the implementation cost. The narrow path is enough to
  ship the Lua-spec hetero use cases the user cares about.

## Consequences

- ~80 LOC HIR + ~400 LOC codegen + ~200 LOC tests.
- 12 new e2e tests, 4 reframed regression tests; total green
  at 781 (= 769 + 12; the four reframed tests stay in the count
  with updated semantics).
- **LIC-2.6a-arr-2 / LIC-2.6a-wr-3 / LIC-2.6b-hash-2 → resolved**
  for Number / Bool / String. Function / Table reservations
  are tracked as new LIC entries to preserve traceability.
- Bool payload's i1 → i64 zext widens the slot's payload bit
  but the storage size stays at 8 bytes (and 8-byte alignment).
- Memory: no change vs ADR 0063 — tagged slots are still 16
  bytes per element / entry.
- `eq` / `ne` between two `Local(TaggedValue)` operands is not
  yet hetero-correct (HIR allows it via `is_number_compatible`
  but the codegen extracts both sides as f64 which traps on
  non-Number). Logged as `LIC-2.6c-tag-hetero-eq-1`.

## Lua-Incompatibility Tracker (cumulative)

Updated from ADR 0063.

| ID | Behaviour | Lua spec | Our behaviour | Status |
|----|-----------|----------|---------------|--------|
| LIC-2.6a-arr-1 | OOB read | returns nil | inline `==nil`: true; locals form: nil-tagged + IsNil / trap on extract | resolved (ADR 0061 + 0063) |
| LIC-2.6a-arr-2 | element kinds | heterogeneous | Number / Bool / String / Nil supported | **resolved (Bool/String, this ADR);** Function/Table → new LIC-2.6c-tag-hetero-fn-tbl-1 |
| LIC-2.6a-arr-3 | key kinds | any | Number+String | partial (ADR 0058) |
| LIC-2.6a-wr-1 | hole write | creates a hole | creates a hole (Nil-tagged) | resolved (ADR 0059) |
| LIC-2.6a-wr-2 | grow write | extends length | extends length | resolved (ADR 0057) |
| LIC-2.6a-wr-3 | array value kinds | heterogeneous | Number / Bool / String supported | **resolved (Bool/String, this ADR);** Function/Table → new LIC-2.6c-tag-hetero-fn-tbl-1 |
| LIC-2.6b-hash-1 | missing key read | returns nil | inline `==nil`: true; locals form: IsNil / trap on extract | resolved (ADR 0061 + 0063) |
| LIC-2.6b-hash-2 | hash value kinds | heterogeneous | Number / Bool / String / Nil-delete supported | **resolved (Bool/String, this ADR);** Function/Table → new LIC-2.6c-tag-hetero-fn-tbl-1 |
| LIC-2.6c-tag-hash-1 | `t.k = nil` | physically removes the key | sentinel + rehash drops | resolved (ADR 0062) |
| LIC-2.6c-tag-locals-1 | `type(x)` for widened local | runtime dispatch on actual tag | static "number" | new (ADR 0063) |
| LIC-2.6c-tag-hetero-fn-tbl-1 | Function / Table table values | accepted | rejected at HIR | **new (this ADR)** |
| LIC-2.6c-tag-hetero-eq-1 | `==` / `~=` between two `TaggedValue` locals | runtime tag-aware | trap when one side is non-Number | **new (this ADR)** |

## Out of Scope

- **Function / Table values in tables** — closure-escape and
  cycle handling required.
- **Heterogeneous `==` between two TaggedValue locals** —
  needs runtime tag dispatch in the BinOp path.
- **String → Number coercion on arithmetic** — Lua spec
  feature, mostly orthogonal.
- **`type(x)` runtime dispatch** for widened locals — still
  static "number".
- **Function-return widening** (`local x = f()` where `f`
  returns nil/heterogeneous) — separate sub-phase.
- **Iteration `pairs` / `ipairs`** — depends on full
  heterogeneous reads, deferred.
