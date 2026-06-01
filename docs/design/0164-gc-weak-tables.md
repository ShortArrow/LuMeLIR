# 0164. Phase 3 GC step 8 — Weak Tables (`__mode`)

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-01
- **Deciders:** ShortArrow

## Context

Step 8 of ADR 0156's roadmap — the last GC ADR. Lua spec §2.5.2: a table whose metatable has `__mode = "k"` has weak keys; `"v"` weak values; `"kv"` both. Weak references don't prevent collection. Decision-only.

## Decision

### Weak-mode discriminator

A bit in the table header's `padding` field (ADR 0156 reserved this slot):

```
TABLE_HEADER_FLAGS at GC_HEADER_OFF (within the GC header's padding)
  bit 0: weak keys   (1 if __mode contains 'k')
  bit 1: weak values (1 if __mode contains 'v')
```

The flag is set when `setmetatable(t, mt)` notices `mt.__mode` is a string containing 'k' or 'v'.

### Mark phase change

In ADR 0159's mark walk for `GC_TYPE_HASH_BUF`:

```
for each occupied entry:
    if weak_keys && key is reference:  // don't push key
    else: push key.payload
    if weak_values && value is reference:  // don't push value
    else: push value.payload
```

The parent table's flag bits are read by checking the table that owns this hash buf — but the hash buf doesn't know its owner. v1 workaround: store the flag bits on the hash buf's GC header padding when allocated. `emit_hash_ensure_buf` and `emit_hash_grow_if_needed` propagate the parent's flags.

### Sweep phase change

After ADR 0161's normal sweep, an additional pass scans every Table whose flag bits indicate weak. For each entry:

```
for each occupied entry:
    if weak_keys && key.tag is reference && key.payload is WHITE:
        // emit tombstone, decrement count
    if weak_values && value.tag is reference && value.payload is WHITE:
        // emit tombstone, decrement count
```

This pass runs BEFORE the WHITE-free pass so the weak-cleared slots don't reference about-to-be-freed memory.

### `__mode` string parsing

At `setmetatable` time, codegen probes `mt.__mode` (similar to `__metatable` per ADR 0140):
- If String containing 'k' → set weak-keys flag.
- If contains 'v' → set weak-values flag.
- Non-string `__mode` → no-op (Lua spec).

The probe is at the setmetatable codegen chokepoint; flags are stored on the table header.

### `__mode` mutation

Lua spec: changing `__mode` after table population is undefined. Our impl reads `__mode` at `setmetatable` time only; subsequent mutations to `mt.__mode` don't change the flag bits. Document this deviation.

## Alternatives considered

- **Ephemeron tables (Lua 5.4 weak-key + value-reachability semantics).** Deferred — non-trivial extension; ship the simpler weak model first.
- **Per-entry weak bits instead of per-table.** Rejected — increases hash entry size, no use case.
- **Read `__mode` at every collection cycle.** Rejected — `__mode` is conceptually static; the spec discourages mutation.

## Consequences

**Positive**
- Standard weak-cache idiom (e.g. memoization tables that don't retain unused entries) works.
- Per-table flag is cheap (already-reserved padding).

**Negative**
- Sweep adds a per-weak-table scan pass.
- Ephemeron tables (Lua 5.4 enhancement) deferred.
- Late `__mode` mutation diverges from Lua spec.

## Future work

- Implementation commit.
- Ephemeron support.
- Weak threads / coroutines (when those land).

## References

- [ADR 0156](0156-gc-architecture-v1.md) — header padding reserved here.
- [ADR 0140](0140-metatable-field-hiding.md) — metatable probe pattern reused for `__mode`.
- [ADR 0161](0161-gc-sweep-phase.md) — sweep extended with the weak-clear pass.
- Lua 5.4 reference manual §2.5.2 — weak tables.
