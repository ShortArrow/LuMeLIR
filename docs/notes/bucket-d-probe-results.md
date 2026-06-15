# Bucket D (deferred §Non-goals) — Probe Results

- **Date:** 2026-06-16
- **Reference:** [leftover roadmap](leftover-roadmap.md) §D
- **State snapshot:** commit `22a4ef4` (ADR 0199 着地後)

## Methodology

Each probable item from leftover-roadmap.md §D was probed via one-shot e2e. Results below.

## Results

| Item | ADR | Status | Evidence |
|---|---|---|---|
| `setmetatable(t, nil)` clear | 0135 | ✅ **既実装** | `setmetatable(t, mt); setmetatable(t, nil); print(getmetatable(t))` → `"nil"` |
| `table.insert(list, pos, value)` arity-3 | 0111 | ✅ **既実装** | `table.insert(t, 2, 99)` shifts and inserts correctly. |
| `table.remove(t, pos)` arity-2 | 0118 | ✅ **既実装** | `table.remove(t, 2)` returns the removed element + shifts. |
| `__tostring` chain (consulted by `tostring()` builtin) | 0142 | ❌ **未実装** | `local mt={}; mt.__tostring=function() return "wrap" end; setmetatable(t, mt); print(tostring(t))` → `"table"` (not `"wrap"`). `tostring()` builtin doesn't consult metamethod. |
| `__concat` metamethod fallback | 0143 | ❌ **未実装** | `t .. "x"` with `mt.__concat` set → `TypeMismatch op=".." lhs="matching kinds" rhs="table vs string"`. HIR kind-checks `..` BEFORE metamethod dispatch. |
| `collectgarbage("setpause", n)` | 0186 | ✅ **RESOLVED by ADR 0200** | Arity (0,1)→(0,2); g_gc_pause mutable global init 200; doubling uses g_gc_pause/100. |

## Verdict

**3 of 6 probed items are ALREADY DONE** — bucket D was overcounted again.

**3 real gaps**:

- D-§0142-tostring-chain — `tostring()` builtin must consult `__tostring` metamethod (Lua §6.1 / §2.4). Small HIR/codegen change.
- D-§0143-concat-metamethod — `..` operator must fall back to `__concat` when operand kinds don't match (Lua §3.4.6). HIR kind-check needs to defer to a runtime metamethod-probe path.
- D-§0186-setpause — `collectgarbage("setpause", n)` and `("stop" | "restart" | "step" | "setstepmul")` extension. Small arity + arm change.

Other bucket D items not directly probed (require deeper investigation):
- 0135 `__metatable` field hiding
- 0103/0104/0105/0113/0152 malloc OOM null-check aggregation (ADR 0157/0184 で partial)
- 0146 `__call` chain
- 0147 算術 metamethod chain
- 0167 multi-hop chain max depth review

## Recommended ordering

1. D-§0186-setpause (smallest, single-builtin arity + arm change)
2. D-§0142-tostring-chain (medium, metamethod wiring inside `tostring()` builtin)
3. D-§0143-concat-metamethod (medium-large, defers HIR kind-check to runtime)

## References

- [leftover roadmap §D](leftover-roadmap.md)
- ADR 0135 — setmetatable (Number-key array `__newindex` resolved here)
- ADR 0142 — `__tostring` metamethod (apparently only partial)
- ADR 0143 — `__concat` metamethod (apparently HIR-level reject still wins)
- ADR 0186 — auto-trigger threshold + factoring
