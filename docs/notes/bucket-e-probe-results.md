# Bucket E (probes / TODOs) — Probe Results

- **Date:** 2026-06-16
- **Reference:** [leftover roadmap](leftover-roadmap.md) §E
- **State snapshot:** commit `7306717` (ADR 0197 着地後)

## Methodology

Each item from `leftover-roadmap.md` §E was probed via a one-shot e2e (compile + run + compare output). Results below.

## Results

| # | Item | Status | Evidence |
|---|---|---|---|
| 1 | Hex literal `0xFF` | ✅ **既実装** | `print(0xFF)` → `"255"`. Lexer ADR 0023 + 0197 + parser arm. |
| 2 | 科学記法 `1e5` | ✅ **既実装** | `print(1e5)` → `"100000"`. Lexer ADR 0023 already covers. |
| 3 | Long strings `[[...]]` | ✅ **既実装** | `print([[hi]])` → `"hi"`. ADR 0038 long-bracket scanner. |
| 4 | Long comments `--[[...]]` | ✅ **既実装** | `--[[ comment ]]\nprint(1)` → `"1"`. ADR 0034 block-comment scanner. |
| 5 | Mid-block return | ✅ **既実装** | `function f() if true then return 1 end; return 2 end; print(f())` → `"1"`. Parser accepts return anywhere in block, HIR + codegen route correctly. |
| 6 | Bracket-key table `{[k]=v}` | ✅ **RESOLVED by ADR 0199** | Parser bracket-key + named-key (`{name=v}`) forms supported; AST/HIR `TableField` enum; keyed fields desugar via IndexAssign pre-statements. |
| 7 | Vararg `...` | ❌ **未実装** | `function f(...) return ... end` → parser: `UnexpectedToken { actual: DotDot, offset: 17 }`. Lexer treats `...` as `..` `.` (DotDot then Dot); no `Ellipsis` token. |
| 8 | `next(t)` arity 1 | ✅ **RESOLVED by ADR 0198** | Was `ArityMismatch`; ADR 0198 relaxed arity (2,2)→(1,2) and HIR synthesizes nil for arg 2. |

## Verdict

**5 of 8 bucket E items are ALREADY DONE** — they were probe-flagged out of caution but the underlying ADRs already covered them. The leftover-roadmap memo overcounted bucket E.

**3 real gaps remain**:

- E6 — bracket-key table constructor (parser-only, ~1 small ADR, 0.5 session)
- E7 — vararg `...` (lexer + parser + HIR + codegen, multi-touch, 1-2 ADR, 1-2 session)
- E8 — `next(t)` arity 1 with implicit `nil` default key (HIR `arity` rule change + lowering, 1 small ADR, 0.5 session)

Updated bucket E count: **3 items / ~1-2 sessions** (down from "7 items / 4-7 sessions" in the original estimate).

## Action items

- [ ] Mark items 1-5 RESOLVED in `leftover-roadmap.md` §E table on next pass.
- [ ] Open per-item ADRs for E6 / E7 / E8 in dependency order:
  - E8 (next arity 1) — smallest, no parser change, good warmup.
  - E6 (bracket-key table) — parser-only, isolated.
  - E7 (vararg) — multi-layer, save for last.

## Conclusion vs the leftover roadmap

Buckets B–F in `leftover-roadmap.md` may also be overcounted; future bucket sweeps should probe-first before scoping ADRs.

## References

- [leftover roadmap §E](leftover-roadmap.md)
- [Lua 5.4 conformance roadmap §Open probes](lua54-conformance-roadmap.md)
- ADR 0023 — numeric literal lexer
- ADR 0034 — block comment scanner
- ADR 0038 — long-bracket string scanner
- ADR 0197 — integer/float token distinction
