# 0258. `<close>` Runtime `__close` Dispatch (N3-D)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-23
- **Deciders:** ShortArrow

## Context

First N3-roadmap sub-ADR. [ADR 0236](0236-const-close-attributes.md) shipped the parser surface for `<close>`, and [ADR 0237](0237-close-readonly-multi-attr.md) wired readonly enforcement (single + multi-binding). The runtime hook — call `mt.__close(v, errobj)` when a `<close>` variable goes out of scope — was explicitly deferred to "M9-C scope". This ADR lands the runtime dispatch for the **normal scope-exit** path.

Per Lua 5.4 §3.3.8:

> When a to-be-closed variable goes out of scope, its `__close` metamethod is called. … The metamethod is called only if the value of the variable is not nil and not false. … Variables are closed in the reverse order to that in which they were declared.

The classic idiom this unblocks:

```lua
local f <close> = io.open("path")   -- (or any setmetatable'd handle)
-- use f
end  -- f's mt.__close fires here, returning the handle to its pool
```

## Scope (literal)

- ✅ A new `is_close: bool` flag on `LocalInfo`; set when the attribute is `"close"` (single-binding `Local` arm + multi-binding `LocalMulti` arm, both parallel and single-Call branches).
- ✅ A new `HirStmtKind::CloseLocals { ids: Vec<LocalId>, candidates: Vec<FuncId> }` variant. Both the `do … end` Block lowering (`StmtKind::Block`) and `lower_scoped_body` / `lower_function_body` append a `CloseLocals` stmt at the end of every scope that introduced one or more `<close>` Locals. `ids` is in **declaration order**; codegen iterates in reverse per spec.
- ✅ `candidates` filters user fns whose declared param kinds and ret kinds match the `__close` call ABI shape (`(Number, Number) → ()`). User fns default to `Number` params when not explicitly widened; the canonical `function(v, e) … end` metamethod matches by construction. Empty candidate set → dispatch path traps `s_call_unknown_fn_ptr` only on a runtime hit (never reached if `<close>` Locals are nil/non-Table).
- ✅ Codegen `emit_close_locals`: for each Table-kind `<close>` Local in reverse declaration order:
  - Load t_ptr; null → skip.
  - Load metatable_ptr; null → skip silently per spec.
  - Probe `mt["__close"]` via `emit_hash_lookup_into_tagged_slot` (ADR 0088 chokepoint).
  - Tag-check TAG_FUNCTION; non-Function → skip silently per Lua spec lenient rule.
  - Dispatch via `emit_dispatch_chain_from_slot_ptr` (ADR 0146 `__call` precedent) with sig `(Number, Number) → ()` and args `(0.0, 0.0)` — the canonical placeholder for the `(value, errobj)` shape. Lua metamethods that ignore their args (the common case) work; those that access `v`/`e` would observe placeholder f64 zeros instead of the real (Table, Nil), see Future work.
- ✅ Skip semantics:
  - Nil / false / non-Table `<close>` values bail at the HIR-kind guard (`matches!(info.kind, ValueKind::Table)`) — no runtime check needed.
  - Missing metatable, non-Function `__close` field both skip silently.
- ✅ Reverse declaration order observable for sibling `<close>` Locals in the same scope (test 3).
- ✅ Function-body close: a `<close>` Local in a user fn closes on natural fall-off (test 2). Reuses `lower_function_body` snapshot point before user body lowering.
- ✅ Block-scope close: `do … end` containing a `<close>` Local closes at `end` (test 1).

## Out of scope

- ❌ **Real `(Table, Nil)` arg pass-through** — the dispatch uses placeholder `(0.0, 0.0)` f64 args matching the Number-default param ABI. Metamethods that access `v`/`e` would see zeros. A future ADR will resolve the dynamic-ABI question (loose match in `compatible_user_functions` vs. widen-at-assignment vs. metamethod-specific tagged ABI).
- ❌ **pcall / error-path close** — when a runtime error unwinds past the scope, `__close` should fire with the error object as second arg. Separate ADR (N3-D-pcall).
- ❌ **Return-path close** — `return` inside a `<close>` scope short-circuits the broken-guard wrap before `CloseLocals` fires. Spec requires close at every scope exit including return; today only natural fall-off works. Separate follow-up.
- ❌ **`goto` / `break` crossing a `<close>` scope** — Lua spec requires close-on-exit for these too. Separate ADR (N3-D-flow).
- ❌ **For-loop body close** — `for i = 1, n do local x <close> = … end` will close on every iteration once the for body's `CloseLocals` insertion is wired through. Trivial extension.
- ❌ **Userdata `<close>`** — N3-C precondition.
- ❌ **Resurrection / re-close** — out of scope per Lua spec corner case.

## Decision

### HIR layer

`LocalInfo` gains `is_close: bool` (initialised `false` in every existing construction site — params, multi-binding, single-binding). The attribute-handling arms in `lower_stmt_match_arms` (single `Local`) and the two `lower_local_multi` paths set `is_close = true` when `attr == Some("close")`. `is_const` continues to be set for both `<const>` and `<close>` per ADR 0237; the new flag is **additive** — `<close>` is now tagged as both const AND close.

`lower_scoped_body` snapshots `self.locals.len()` on entry, lowers the body, then walks the newly-introduced range (start..end) for any `is_close = true` Local. If the set is non-empty, it appends a `HirStmtKind::CloseLocals { ids }` stmt at the end of the lowered body — `ids` in declaration order. The same hook fires for function bodies, since the function lowerer also uses `lower_scoped_body` for the top-level block.

### Codegen layer

A new `emit_close_locals` helper takes `&[LocalId]`, iterates in **reverse**, and for each id:

1. Loads `slots[id.0]` as a TaggedValue (i64 tag + i64 payload).
2. Computes `should_dispatch = tag != TAG_NIL && tag != TAG_FALSE && tag == TAG_TABLE` (an `&&` chain reduced to two `cmpi` + `and`).
3. Inside an `scf.if(should_dispatch)`:
   - Bitcasts payload to ptr → `t_ptr`.
   - Loads `metatable_ptr = t_ptr[+32]` (TABLE_OFF_METATABLE).
   - Inside `scf.if(mt_ptr != null)`:
     - Reuses **a new sibling helper** `emit_probe_metatable_field_named(field_global_name)` — the existing `emit_probe_metatable_field` is `__metatable`-only; this sub-ADR extracts the field-global parameter so `__close` (and future `__gc`, `__mode`) can share the probe path. (Tidy First refactor.)
     - Tag-checks the probe slot for `TAG_FUNCTION`. If yes, calls through the closure-cell dispatch helper introduced by ADR 0146 (the `__call` codepath that already handles "Function value loaded from a slot, call it with (Table, Nil)").

The second-argument (errobj) builder pushes a Nil TaggedValue slot — no error object on normal-exit close.

### String global

A new constant string `s_close_field_name = "__close\0"` is added to the metamethod-field-name pool (alongside `s_metatable_field_name`, `s_tostring_field_name`, etc.).

### LocalInfo construction sites

Three sites in `src/hir/mod.rs` build `LocalInfo` directly (rather than going through `declare_local`): param construction at function boundary (2 sites) and one synthetic-local builder. All three gain `is_close: false`. `declare_local` itself already routes through a default builder — wire the flag through.

## Why this ABI

`compatible_user_functions` (ADR 0082) matches user fns by exact-equality of declared param and ret kinds. User fn params default to `Number` and ret_kinds default to empty when not refined by a call site. The canonical `__close` metamethod shape `function(v, e) print(...) end` matches `(Number, Number) → ()` by construction. Dispatching `__close` with placeholder `(0.0, 0.0)` f64 args satisfies the ABI and lands working invocation for the common case (metamethods that ignore their args — release locks, return handles to a pool, fire side-effects).

The real `(Table, Nil)` arg pass-through is a separate dynamic-typing question that affects every metamethod call ABI, not just `__close`. Resolving it deserves a dedicated ADR with alternatives compared (relax the matcher vs. widen at assignment vs. metamethod-tagged ABI). Today's choice keeps the codepath narrow and additive — no existing dispatch matchers change.

## Why a synthetic stmt, not a special `Block` field

`HirStmtKind::Block { stmts }` is structurally pure. Adding a `close_ids: Option<Vec<LocalId>>` field there would force every Block consumer to consider close-locals, even though 99% of Blocks have none. A synthetic `CloseLocals` stmt is **additive** — every existing pass treats it as opaque until it's specifically taught. Codegen is the only consumer that matters today.

## Why reverse order matters

The Lua-canonical idiom is matched-pairs:

```lua
local a <close> = open_a()
local b <close> = open_b()  -- b depends on a being still-open
```

Closing `b` first releases the b-side first, leaving a alive while b's `__close` runs. Closing a first would invert the dependency. Spec is explicit and CPython / official Lua both do it this way.

## Tests

`tests/phase4_n3d_close_runtime.rs` (NEW, 5 e2e, all Green):

1. **Single close fires on Block exit** — `do local x <close> = setmetatable({}, mt) end` runs `mt.__close` exactly once.
2. **Single close fires on function-body fall-off** — same shape inside `local function f() … end; f()`.
3. **Reverse-order close** — two `<close>` Locals; their side effects appear in reverse declaration order.
4. **Nil-valued `<close>` skips** — `local x <close> = nil` does not trap; the metamethod is never probed.
5. **Non-Table `<close>` skips silently** — `local x <close> = 42` does not trap and does not dispatch.

`tests/phase4_m9_const_attribute.rs` and `tests/phase4_m9_close_readonly_multi.rs` continue to gate the parser + readonly surface unchanged.

## Test count delta

```
Step 0:  1641 (after ADR 0257)
N3-D (impl + 5 new e2e):  1641 → 1646
```

## What this unblocks

- The `io.open` / `io.lines` deterministic-cleanup idiom (once `io.open` lands).
- The "RAII-style" Lua patterns (lock guards, transaction handles) that spec authors expected when adopting `<close>` into 5.4.
- N3-A `__gc` (ADR 0259, planned) will reuse the metatable-field probe infrastructure.

Still gated:
- pcall / error-path close (N3-D-2, separate sub-ADR).
- `goto` / `break` exit-edge close.
- `for` / `while` / `repeat` body close (mechanical extension).

## References

- [ADR 0236](0236-const-close-attributes.md) — `<close>` parser surface + `<const>` readonly.
- [ADR 0237](0237-close-readonly-multi-attr.md) — `<close>` readonly; this ADR closes the runtime gap.
- [ADR 0088](0088-table-hash-lookup-chokepoint.md) — hash-lookup chokepoint reused for the probe.
- [ADR 0146](0146-call-metamethod.md) — closure-cell call ABI dispatched on the probed Function.
- [ADR 0163](0163-gc-finaliser.md) — `__gc` strategy; N3-A will reuse N3-D's probe path.
- [Lua 5.4 §3.3.8](https://www.lua.org/manual/5.4/manual.html#3.3.8) — to-be-closed variables.
- [Roadmap rebuild 2026-06-21](../notes/roadmap-2026-06-21-rebuild.md) — N3-D in the N1-N10 path.
