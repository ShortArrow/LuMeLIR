# AGENTS.md — LuMeLIR Working Conventions for LLM Coding Agents

> Primary audience: LLM coding agents (Claude Code, OpenAI Codex, Cursor, Aider, Devin, ...).
> Human contributors: see [CONTRIBUTING.md](CONTRIBUTING.md) first, then come back here for details.

## 0. About This Document

- **Single source of truth** for working conventions in this repository.
- `CLAUDE.md` and `CONTRIBUTING.md` are thin pointers — do not duplicate content there.
- If this file exceeds ~350 lines, split details into `docs/agents/*.md` and keep this file as the index.
- Update this file in the same commit as any policy change (see §9).

## 1. 30-Second Project Summary

LuMeLIR is a Rust-based compiler toolchain that lowers Lua through **MLIR** into native AOT binaries for heterogeneous targets (CPU / GPU / FPGA / MCU). The thesis: **Lua as a frontend for MLIR's transformation engine**, not merely a scripting language.

Full product requirements: [`docs/PRD.jp.md`](docs/PRD.jp.md) (Source of Truth, Japanese) / [`docs/PRD.md`](docs/PRD.md) (English translation).

## 2. Current Phase Status

| Phase | Status | Scope |
|---|---|---|
| Phase 0 — Scaffolding | **Done** | Cargo workspace, CLI skeleton (clap), docs, dual license, ADR conventions |
| Phase 1 — PoC | **Done** | `print(1 + 2)` AOT: lexer → parser → MLIR emit → native binary (ADR 0006) |
| Phase 2 — Core Semantics | **In progress** | `local`, scopes, control flow, tables, metatables, GC |
| ‣ 2.0 `local` + multi-stmt | **Done** | HIR layer introduced; `local x = 1; print(x + 2)` (ADR 0007) |
| ‣ 2.0a auto-declare globals (top-level only) | **Done** | bare `x = 1` at chunk scope auto-declares as chunk-level local; type-stable; no cross-function leak (ADR 0048) |
| ‣ 2.1 reassignment / scopes | **Done** | `x = 2`, `do ... end` blocks, scope stack, shadowing (ADR 0008) |
| ‣ 2.1a multi-target reassignment | **Done** | `a, b = b, a` parallel evaluation via temp-then-assign; auto-declare via ADR 0048 (ADR 0049) |
| ‣ 2.1b multi-target reassign from Call | **Done** | `a, b = pair()` reuses `MultiAssignFromCall` HIR node; targets auto-declare per ADR 0048 (ADR 0050) |
| ‣ 2.2a arithmetic operators | **Done** | `-` `*` `/` `%` `^` + unary `-`; libm pow/floor (ADR 0009) |
| ‣ 2.2b comparisons + bool literals | **Done** | `<` `<=` `==` `~=` `>` `>=`, `true`/`false`; ordered cmpf, print(bool) (ADR 0010) |
| ‣ 2.2c floor div + bitwise ops | **Done** | `//`, `&`/`\|`/`~`/`<<`/`>>`, unary `~`; f64↔i64 via fptosi/sitofp (ADR 0022) |
| ‣ 2.2d hex / float / scientific literals | **Done** | `0xff`, `3.14`, `1e3`, `2.5e-1`; lexer-only change (ADR 0023) |
| ‣ 2.3a nil + per-slot types + heterogeneous == | **Done** | `nil`, `local b = true`, `1 == nil` → false (ADR 0011) |
| ‣ 2.3b control flow | **Done** | `if`/`elseif`/`else`/`while` via `scf`, truthiness helper (ADR 0012) |
| ‣ 2.3c short-circuit | **Done** | `and`/`or`/`not` via `scf.if` expression form + `arith.xori` (ADR 0013) |
| ‣ 2.3d numeric for | **Done** | `for i=s,e[,step] do ... end` via `scf.while` desugar + read-only loop var (ADR 0014) |
| ‣ 2.4 break | **Done** | `break` via HIR-time desugar to hidden `_broken` flag + body guard wrap (ADR 0015) |
| ‣ 2.4b `repeat ... until` | **Done** | do-while loop, until-cond sees body locals, scf.while body-in-`before` (ADR 0035) |
| ‣ 2.5a top-level functions | **Done** | `local function`, `return`, recursion (Number-only params/ret) (ADR 0016) |
| ‣ 2.5b anonymous + first-class (HIR-time) | **Done** | `local f = function() end`, alias `local g = f`, static dispatch (ADR 0017) |
| ‣ 2.5b.2 functions as args | **Done** | `apply(f, x)`, `func.call_indirect`, param-kind back-inference (ADR 0018) |
| ‣ 2.5b.3 functions as return values | **Done** | `return f`, ret_kind→Function, ptr-slot+ucast bridging (ADR 0019) |
| ‣ 2.5e Bool/Nil params/return | **Done** | predicates (`return x > 0`), `not b`, `nil`-returning helpers; call-site param inference (ADR 0020) |
| ‣ 2.5f nested `local function` (no capture) | **Done** | sibling forward-reference + recursion via shared register/lower helpers (ADR 0036) |
| ‣ 2.5c-min capture-by-value closures | **Done** | Number upvalues, direct-call only, MLIR signature widens to `[params + upvalues]` (ADR 0037) |
| ‣ 2.5c.1 top-level `local function` captures chunk locals | **Done** | Pass 2 interleaved with main chunk walk; `idx_of_funcdef` removed (ADR 0042) |
| ‣ 2.5c.2 Bool / Nil / String upvalue captures | **Done** | one predicate flip in `lookup_or_capture_upvalue`; codegen unchanged (ADR 0043) |
| ‣ 2.5c.3 closure-escape static rejection | **Done** | `HirError::ClosureEscapes` for closures-with-upvalues used as args / returns (ADR 0044) |
| ‣ 2.9a line/column diagnostics | **Done** | CLI renders errors as `path:line:col: <layer> error: …` via `cli::diag` (ADR 0045) |
| ‣ 2.9b source-snippet caret display | **Done** | `format_error` appends a rustc-style two-line snippet via `snippet` pure helper (ADR 0046) |
| ‣ 2.9c strip offset from error Display | **Done** | Tidy First — drop redundant `at byte offset N` / `(offset N)` from variant Display strings (ADR 0047) |
| ‣ 2.5d multi-return | **Done** | `return a, b`, `local x, y = call()`, parallel binding, multi-result `func.call` (ADR 0021) |
| ‣ 2.7a string literals + `#` | **Done** | `"..."`/`'...'`, basic escapes, `print(s)`, `#s` via strlen, deduped LLVM globals (ADR 0024) |
| ‣ 2.7b string concat / equality | **Done** | `a..b` via malloc+memcpy, `s1 == s2` via strcmp, call-site String inference (ADR 0025) |
| ‣ 2.7c `tostring` + concat auto-coerce | **Done** | `tostring(x)` builtin, `"x"..1`/`..true`/`..nil` desugar via tostring (ADR 0026) |
| ‣ 2.7d lexicographic string compare | **Done** | `<` `<=` `>` `>=` for String operands via strcmp (ADR 0027) |
| ‣ 2.7e `tonumber` (NaN sentinel) | **Done** | `tonumber(n)` identity, `tonumber(s)` via sscanf, NaN on parse fail (ADR 0028) |
| ‣ 2.7f `type(x)` | **Done** | static kind→typename ptr, Function values admissible (ADR 0029) |
| ‣ 2.7g `assert(cond)` | **Done** | Bool-only assert with libc exit(1) on failure (ADR 0030) |
| ‣ 2.7m `assert(cond, msg)` | **Done** | optional 2nd-arg String message routed into the failure printf (ADR 0051) |
| ‣ 2.7n `tostring(f)` for Function | **Done** | returns the literal `"function"` via shared `s_typename_function` global (ADR 0052) |
| ‣ 2.7h `error(msg)` | **Done** | unconditional failure via shared `emit_exit_with_message` helper (ADR 0033) |
| ‣ 2.8a single-line comments | **Done** | `-- ...` skipped by lexer (ADR 0031) |
| ‣ 2.8b variadic `print` | **Done** | `print()`/`print(a)`/`print(a, b, ...)` with `\t` separator + `\n` (ADR 0032) |
| ‣ 2.8c block comments | **Done** | `--[[ ... ]]` multi-line via `skip_block_comment` helper (ADR 0034) |
| ‣ 2.8d `#!` shebang line | **Done** | leading `#!` skipped to first newline at byte 0 (ADR 0041) |
| ‣ 2.7j long-bracket strings + level-N block comments | **Done** | `[==[ ... ]==]` and `--[==[ ... ]==]` via shared `scan_long_bracket_body` (ADR 0038) |
| ‣ 2.7k extended string escapes | **Done** | `\a \b \f \v \xHH \ddd` (ASCII range) via `read_hex_escape` / `read_decimal_escape` (ADR 0039) |
| ‣ 2.7l `\u{XXXX}` + `\z` | **Done** | Unicode codepoint → UTF-8 via `read_unicode_escape`; `\z` skips whitespace run (ADR 0040) |
| ‣ 2.5c closures | Not started | upvalue capture, heap-allocated environments |
| ‣ 2.6a-min empty tables `{}` + `#t` | **Done** | `ValueKind::Table` (`!llvm.ptr`), `[i64 length]` heap header, malloc on construct (ADR 0053) |
| ‣ 2.6a-arr Number array constructor + `t[i]` read | **Done** | `{e1,e2,…}` populated form, integer indexing, runtime OOB trap (ADR 0054) |
| ‣ 2.6a-wr Number array element write `t[i] = v` | **Done** | parse-then-equals fallthrough, `IndexAssign` AST/HIR, codegen mirrors read path (ADR 0055) |
| ‣ 2.6a-norm stable table header (Tidy First) | **Done** | 32-byte header + separate array_buf; frozen offsets at 0 (length) / 16 (array_buf); alias-safe under grow (ADR 0056) |
| ‣ 2.6a-grow array push `t[#t+1] = v` | **Done** | doubling capacity + realloc inside stable header; alias-safe under grow; LIC-2.6a-wr-2 resolved (ADR 0057) |
| ‣ 2.6b-hash string-keyed `t.k` / `t["k"]` | **Done** | open addressing + linear probing on `hash_buf`; FNV-1a hash; doubling rehash; sugar parser-level (ADR 0058) |
| ‣ 2.6c-tag-arr tagged array slots + holes | **Done** | 16-byte `{tag, value}` slots; `t[#t+2]=v` hole creation with Nil-tagged gap fill; LIC-2.6a-wr-1 resolved (ADR 0059) |
| ‣ 2.6c-tag-hash tagged hash entries + `t.k = nil` | **Done** | 24-byte hash entries (`{ptr key, 16-byte value slot}`); soft-delete via Nil tag; LIC-2.6b-hash-2 partial (Nil accepted) (ADR 0060) |
| ‣ 2.6c-isnil-query inline `t[i] == nil` / `t.k == nil` non-trapping | **Done** | HIR pattern detection before fold → `IsNilQuery`; non-trapping codegen (OOB / missing key / Nil tag → true); LIC-2.6a-arr-1 + LIC-2.6b-hash-1 partial (inline form only) (ADR 0061) |
| ‣ 2.6c-tag-hash-hard hash hard tombstone | **Done** | `t.k = nil` overwrites key with `HASH_DELETED_KEY=1` sentinel; probe helpers skip past it; rehash physically drops sentinel entries; LIC-2.6c-tag-hash-1 resolved (ADR 0062) |
| ‣ 2.6c-tag-locals Number-MaybeNil locals widening | **Done** | `local x = t[i]` widens x into a 16-byte tagged slot (`{tag, f64}`); `if x == nil` lowers to non-trapping `IsNilLocal`; LIC-2.6a-arr-1 + LIC-2.6b-hash-1 resolved for the locals form (ADR 0063) |
| ‣ 2.6c-tag-hetero heterogeneous Bool/String table values | **Done** | TAG_BOOL/STRING extend the tagged slot; `{1, "hello", true}` and `t.k = "world"` accepted; `print(Local(TaggedValue))` runtime tag dispatch; LIC-2.6a-arr-2 / LIC-2.6a-wr-3 / LIC-2.6b-hash-2 resolved for Bool/String (ADR 0064) |
| ‣ 2.6c-tag-hetero-fix inline print + Eq dispatch | **Done** | codex-review-flagged P1: `print(t[k])` materialises through tmp tagged slot for runtime tag dispatch; `TaggedValue == literal` lowers to runtime tag-check + per-kind compare instead of fold; supersedes ADR 0061/0063 plain-read-trap claims; LIC-2.6c-tag-hetero-inline-1 resolved (ADR 0065) |
| ‣ 2.6c-tag-hetero-eq IsNil unification + Local-Local `==` | **Done** | Tidy First: `IsNilQuery` + `IsNilLocal` collapse into `IsNil(Box<HirExpr>)`. Feature: `Local(TaggedValue) == Local(TaggedValue)` runtime tag-vs-tag dispatch + per-kind compare (cmpf / cmpi / strcmp). LIC-2.6c-tag-hetero-eq-1 resolved (ADR 0066) |
| ‣ 2.6c-tag-consumers `type` / `tostring` runtime dispatch | **Done** | `type(Local(TaggedValue))` and `tostring(Local(TaggedValue))` route through new helpers that read the slot tag at runtime; concat (`..`) auto-coerce reuses the new tostring path; matrix-test scaffold introduced; LIC-2.6c-tag-locals-1 resolved (ADR 0067) |
| ‣ 2.6c-tag-doc-consolidate tagged-semantics SoT | **Done** | `docs/design/tagged-semantics.md` introduced as the SoT for TaggedValue slot layout, producer/source taxonomy, consumer coverage matrix, runtime invariants, consolidated LIC table; future ADRs delegate LIC tracking to the doc instead of duplicating tables (ADR 0068) |
| ‣ 2.6c-tag-defensive-trap unknown-tag fail-fast | **Done** | `emit_tagged_unknown_tag_trap` replaces silent `else` fallbacks in `emit_type_tagged_local` / `emit_tostring_tagged_local` / `emit_tagged_eq_local_local` / `emit_print_tagged_local` (Function/Table reserved tag); trap unreachable today (HIR rejects), guard rail for the day reserved tags ship (ADR 0069) |
| ‣ 2.6c-tag-consumers-inline `type(t[k])` / `tostring(t[k])` | **Done** | `Builtin::Type` / `Builtin::ToString` arm gain `HirExprKind::Index` special case mirroring the ADR 0065 print pattern (tmp tagged slot via `emit_local_init_tagged` + dispatch via `emit_type_tagged_local` / `emit_tostring_tagged_local`); `..` concat auto-coerce inherits the new dispatch; LIC-2.6c-tag-consumers-inline-1 resolved (ADR 0070) |
| ‣ 2.6c-tag-fn-tbl Function / Table values in tables | **Done** | TAG_FUNCTION=4 / TAG_TABLE=5 wired up; `_store_function` / `_store_table` helpers; HIR `value_ok` matrix opens closure-less Function and Table values (closure-with-upvalues stays HIR-rejected); 4 consumer dispatch chains extended; rule-of-three Tidy First extracts `emit_inline_index_into_tagged_tmp`; LIC-2.6c-tag-hetero-fn-tbl-1 resolved, partial trio (arr-2/wr-3/hash-2) promoted to resolved, two new pending LIC entries logged (ADR 0071) |
| ‣ 2.6c-tag-fn-tbl-call call through tagged slot | **Done** | `lower_call` accepts `Local(TaggedValue)` as `Callee::Indirect`; codegen `Callee::Indirect` arm gets a TaggedValue branch via new `emit_value_slot_check_function` trap helper, reconstructs `(f64,…) → f64` from `args.len()`; LIC-2.6c-tag-hetero-fn-tbl-call-1 resolved (ADR 0072) |
| ‣ 2.6c-tag-rs-split codegen module split | **Done** | 2-layer split: new `src/codegen/primitive.rs` (pure MLIR + `Types` + libc-call shells, ~344 LOC) and `src/codegen/tagged.rs` (tag constants, store/check helpers, pure-tag consumer dispatchers `print` / `type` / `eq Local-Local`, ~1337 LOC). emit.rs 8464 → 6856 LOC. Statement-context tagged materializers stay in emit.rs (recurse through `emit_expr`); HIR-coupled refactor deferred (ADR 0073) |
| ‣ 2.6c-tag-locals-fn function-return widening | **Done** | HIR `lower_return_with_values` widens `_ret_value_N` to TaggedValue when same return position sees mixed kinds; `_ret_value_N` Nil-init for empty exits; codegen `ret_mlir_types` emits `(i64 tag, i64 payload_raw)` for each TaggedValue position; new `emit_call_user_into_tagged_slot` / `_tmp` helpers wire LocalInit/Assign and inline Print/Type/ToString consumers; HIR rejects storing tagged-return functions in tables (LIC-2.6c-tag-locals-fn-indirect-1 backstop). LIC-2.6c-tag-locals-fn-1 resolved; 3 new pending LICs logged (ADR 0074) |
| ‣ 2.6c-tag-shape-tests + dispatch-preamble | **Done** | Tidy First post-ADR-0074: 3 MLIR-shape tests pin the `(f64) -> (i64, i64)` widened-return ABI; new `emit_tag_and_payload_ptr` helper in `tagged.rs` collapses the tag-load + payload-ptr preamble in 3 dispatchers (print / eq / tostring). Callback-based skeleton extraction confirmed infeasible (Rust borrow-checker vs melior eager region build). 869 → 872 green, no ADR (refactor only) |
| ‣ 2.6c-tag-callee-arity tagged-callee arity hardening | **Done** | Strict Plan C: HIR rejects every indirect call through a TaggedValue local via new `HirError::IndirectCallThroughTaggedLocal`. ADR 0072's `local g = t[k]; g()` pattern rolled back — `args.len()` reconstruction was unsound on heterogeneous-arity / heterogeneous-return tables. Codegen drops the TaggedValue branch in `Callee::Indirect`; `emit_value_slot_check_function` deleted. 5 new tests pin the new safety boundary, 6 ADR 0072 tests reframed to negative reject assertions, 1 deleted (runtime trap path now unreachable). LIC-callee-arity-1 + locals-fn-indirect-1 resolved; hetero-fn-tbl-call-1 status revisited as "resolved by removal". 872 → 876 green (ADR 0075, supersedes ADR 0072 in part) |
| ‣ 2.6c-tag-locals-fn-multi multi-position widening | **Done** | Caller-side result-index walker generalised: new `ret_kind_result_width` / `flat_result_index` pure helpers + `emit_pack_tagged_result_at_pos` pack helper. `emit_multi_assign_from_call` now per-position dispatches via `flat_result_index`, packing TaggedValue positions through the new helper. `(i64, i64, i64, i64)` MLIR signature for two TaggedValue positions is shape-tested. HIR widening was already per-position-correct (ADR 0074), `ret_mlir_types` flat_map already multi-position-ready — no HIR change needed. 11 new e2e tests + 1 MLIR-shape test. LIC-locals-fn-multi-1 resolved; 17/1/1. 876 → 888 green (ADR 0076) |
| ‣ 2.7p-arith-string-coerce string→number arith coercion | **Done** | Lua spec §3.4.1: arithmetic / bitwise BinOps auto-coerce String operands. New HIR `ArithStringCoerce` wrapper (variant + infer_kind arm + `coerce_arith_operand_if_string` helper) rewrites String operands to satisfy the existing `is_number_compatible` check. Codegen `emit_tonumber_for_arith` reuses `emit_tonumber`'s sscanf path then traps via `s_arith_coerce_failed` on NaN — distinct from `Builtin::ToNumber`'s NaN-sentinel contract (ADR 0028). 12 ops (`+ - * / // % ^ & \| ~ << >>`) accept String operands; hex floats work via glibc sscanf%lf. 10 new e2e tests, 888 → 898 green. LIC-arith-coerce-1 resolved; arith-coerce-tagged-1 added pending. 18/1/2 (ADR 0077) |
| ‣ 2.8e-iter-ipairs ipairs sugar | **Done** | Plan C (Codex post-ADR-0077): `for k, v in ipairs(t) do … end` is recognised at the parser level only. New `Keyword::In`, `StmtKind::ForIpairs` AST variant, `parse_for` branches on `,` for sugar form, `unwrap_ipairs_call` restricts the iter slot to `ipairs(table)`, `ParseError::UnsupportedIterator` for `pairs(t)` and arbitrary iters. HIR `lower_stmt(ForIpairs)` desugars to `Block { LocalInit __t; LocalInit idx=1; LocalInit broken=false; While(true) { LocalInit val=__t[idx]; If IsNil(val) then broken=true else BODY; idx += 1 } }` using `IndexTagged` (ADR 0063) for non-trapping reads and `lower_scoped_body_no_push` for break-flag wrapping (ADR 0015). Codegen unchanged. 10 new e2e tests, 898 → 908 green. LIC-iter-ipairs-1 resolved; iter-pairs-1 / iter-generic-1 added pending. 19/1/3 (ADR 0078) |
| ‣ 2.6b-hash-keys hash key kinds expansion | **Done** | Plan E tagged-key (Codex post-ADR-0078): hash entry widened 24→32 bytes with `{16-byte tagged key, 16-byte tagged value}`. New `TAG_DELETED = 6` retires the `HASH_DELETED_KEY = 1` ptr sentinel; tombstones now live in the key tag word. New codegen helpers `emit_build_search_key_slot`, `emit_hash_key_hash_dispatched` (FNV-1a for String, `× FNV_PRIME` of the i64 payload word for Number / Bool / Function / Table), `emit_hash_key_eq_dispatched` (tags-equal gate then per-tag payload compare; `cmpf Oeq` for Number, strcmp for String, raw i64 cmpi for the rest). HIR `is_hash_key_eligible` accepts Number / String / Bool / Function / Table; nil keys still HIR-rejected. Probe loop refactored to take a tagged search-key slot ptr. Rehash copies the 16-byte tagged key raw. 12 new e2e tests + 2 reframed regression tests, 908 → 920 green. LIC-2.6a-arr-3 resolved (was partial); 2 new pending runtime-diag LICs (`hash-key-nil-runtime-1`, `hash-key-nan-runtime-1`). 20/0/5 (ADR 0079) |
| ‣ 2.8e-iter-pairs pairs hash iteration | **Done** | Plan A' (Codex post-ADR-0079): `for k, v in pairs(t) do … end` ships as parser sugar (sibling of ipairs) with codegen-owned dual-phase walker. New `StmtKind::ForPairs` / `HirStmtKind::ForPairs` opaque shapes; new `emit_for_pairs` walks array part 1..=len then hash part 0..cap with `TAG_NIL` (empty / array hole) and `TAG_DELETED` (tombstone) skip. Rehash safety (Codex P1): per-iteration `header.hash_buf` / `header.array_buf` reload + ptr-equality detect aborts the loop on body-driven `emit_hash_grow_if_needed`. New `emit_copy_value_slot_16b` helper consolidates the rehash-migration copy pattern; key slot for array phase built via `emit_value_slot_store_number`. 16 new e2e tests (sorted-output for hash-coverage per Codex P2, `type(k)` materialization for all 5 key kinds per Codex P2 #4) + 1 obsolete reject test removed, 920 → 935 green. LIC-2.8e-iter-pairs-1 resolved; new pending LIC-2.8e-pairs-tagged-key-write-1 (TaggedValue-key IndexAssign HIR-rejected). 21/0/4 (ADR 0080) |
| ‣ 2.8e-iter-next next builtin + ForPairs HIR-desugar | **Done** | Plan Alpha (Codex post-ADR-0080, restricted scope vs Plan B Beta superseder of ADR 0075). `Builtin::Next` is the first multi-return builtin: `Builtin::ret_kinds()` + `MultiAssignFromCall(Callee::Builtin)` open the path so `local k, v = next(t, c)` works. Module-level `@__lumelir_next(t, prev_tag, prev_payload) → (i64×4)` (ADR 0076 flattened ABI for two TaggedValue positions); body is a stateless linear scan with a `found` flag — naive O(N) per call, O(N²) per pairs loop, acceptable for typical tables. ForPairs HIR-desugars to `Block + LocalInit + While + MultiAssignFromCall + If + Assign` using existing primitives; ~707 LOC of codegen deleted (`emit_for_pairs` + 4 helpers + `emit_copy_value_slot_16b`), ~750 LOC added. 5 new e2e in `tests/phase2_8e_next.rs`, 16 ADR 0080 ForPairs e2e regress green, 935 → 940. LIC-2.8e-iter-pairs-1 resolution mechanism updated (ADR 0080 → ADR 0081); new resolved LIC-2.8e-builtin-multi-return-1. 22/0/4 (ADR 0081) |
| ‣ 2.5x-callee-dispatch general indirect-call re-enablement | **Done** | Plan B3 (Codex post-ADR-0081, supersedes ADR 0075 in part). New `Callee::IndirectDispatch { local_id, sig: IndirectSig, candidates: Vec<FuncId> }` joins existing `Callee::Indirect(LocalId)` (parameter calls retain the safe direct path). HIR `lower_call` filters user fns by `param_kinds` only and picks the first match's `ret_kinds` as canonical; `lower_local_multi` / `lower_assign_multi` re-filter for multi-value position with `names.len()`-aware ret_kinds. New codegen `emit_indirect_dispatch_call` runs (1) tag check vs `TAG_FUNCTION` (Codex P3, must precede payload interpretation), (2) ptr load, (3) nested `scf.if` chain comparing loaded ptr to each candidate's `func.constant @user_fn_X` and emitting **direct** `func.call @user_fn_X(args)` — never `func.call_indirect` cast (forward-edge integrity, Codex §4). Multi-value path reuses `flat_result_index` (ADR 0076). New `src/codegen/callabi.rs` extracts `ret_mlir_types` / `ret_kind_result_width` / `flat_result_index` (Tidy First). New runtime traps `s_call_non_function` / `s_call_unknown_fn_ptr`. New `IndirectCallNoCandidates` HIR error for compile-time empty-candidate detection. 11 reframed tests (ADR 0072/0075 reject → positive) + 4 new e2e (multi-return indirect, closure-escape regression, no-candidates compile error, same-sig dispatch). 940 → 944 green. LIC-2.6c-tag-hetero-fn-tbl-call-1 reframed "resolved by safe static dispatch"; new resolved LIC-2.5x-callee-dispatch-1. 23/0/4 (ADR 0082) |
| ‣ 2.5c-full full closures (Plan B) | **Commit 2a / 2a-fix / 2b / 3a / 3b prep / 3b prep fix / 3b body / 3c landed** | ADR 0083 plan landed via Codex pre-review; Commit 1 (`e6b256f`) shipped `src/codegen/closure.rs` skeleton. MLIR feasibility spike (2026-05-07): A1/B1/B2/B4/B5b PASS. **Commit 2a** (`551d51c`): `emit_function` / `emit_main` / `emit_lumelir_next_function` → `LLVMFuncOperationBuilder`; multi-return → `!llvm.struct<(...)>`. **Commit 2a-fix** (`c81f16b`): HIR reject of non-Number ret_kinds on `Callee::Indirect` (ADR 0075 amend, lifts in future ADR — Function-kind upvalue support; ADR 0087 / 0088 were claimed for hash-key validity / hash-lookup-miss work on 2026-05-10). **Commit 2b** (`a5e8a3e`): per-fn `@user_fn_NN_closure` singletons; producer + consumer flip; `emit_load_closure_fn_ptr` consumer normalisation. **Commit 3a** (`20e563e`): closure.rs 6 capturing helpers + `LocalInfo::is_captured` + `HirFunction::parent_scope` + post-pass. **Commit 3b prep** (`e8db350`): `Callee::User` struct variant `{ fid, holding_local: Option<LocalId> }` + `emit_call_user_with_cell` helper (`#[allow(dead_code)]`). **Commit 3b prep fix** (`f2ffcb9`, post-Codex-review): synthetic local for FunctionDef + post-pass `MutualCapturingRecursion` reject; local-resolve path's per-arg Function(arity) compat check (Codex P1 `holding_local` 3-way ambiguity resolved). **Commit 3b body atomic** (`18bee17`, 2026-05-10): every user `llvm.func` now accepts `!llvm.ptr` cell ptr as first arg; entry block unpacks `cell.upvalue_box[i]` for each upvalue; 4 direct-call sites (`Call` expr, multi-assign, dispatch chain then-branch, TaggedValue pack helper) route through `emit_call_user_with_cell`; `FunctionRef` allocates fresh capturing cells; `Local`-known-FuncId branches on `target.upvalues.is_empty()` (singleton vs slot-load); LocalInit storage rule stores cell ptr unconditionally for capturing targets; outer-scope `is_captured` locals get heap upvalue boxes at function entry. `f2ffcb9` 980 → `92e3f4f` 980 → 3b body 984. 4 new IR-shape tests pin entry cell_ptr / self-recursion / nested forward / alias paths. **Commit 3c**: removed all 5 `HirError::ClosureEscapes` reject sites + `closure_with_upvalues` helper + `f.upvalues.is_empty()` generic-for filter; `Callee::Indirect` and the dispatch chain then-branch now thread the loaded cell ptr (not `cell.fn_ptr`) as `in_function_cell_ptr` so capturing closures reach their boxes when reached through tagged-slot escape paths; 7 new e2e tests in `phase2_5c3_capturing_e2e.rs` (box_sharing / make_adder / closure_return / table_capture / closure_identity / generic_for_capturing / IR-shape hardening); 7 negative escape tests across 6 files inverted to positive lowering pins. 984 → 990. LIC-2.6c-tag-hetero-closure-escape-1 resolved. ADR 0044 superseded by ADR 0083 in full. (ADR 0083) |
| ‣ 2.8e-iter-tk TaggedValue-key IndexAssign + Index read | **Done** | Codex (C) pivot — ADR 0083 deferred, ADR 0084 先行. HIR `is_hash_key_eligible` accepts `ValueKind::TaggedValue` (1-line relax). Codegen runtime tag dispatch in IndexAssign / Index: tag check first (`TAG_NIL` trap via new `s_table_index_nil`, Lua spec §3.4.5), pin the local's slot as the search-key slot directly (no fresh `emit_build_search_key_slot` tmp), hash probe via the existing ADR 0079 dispatched helpers. New-key commit copies the 16-byte search slot into `entry+0` raw (no kind-aware store needed). Resolves the natural `for k, v in pairs(t) do t[k] = v + 100 end` idiom; ADR 0080's `pairs_body_writes_separate_table_safely` workaround reframed to `pairs_body_mutates_existing_value_safely`. 7 new e2e in `tests/phase2_8e_tagged_key_indexassign.rs`. Array path bypassed for TaggedValue keys (documented limitation: Number-keyed reads still see array slot, not hash mirror). 944 → 951 green. LIC-2.8e-pairs-tagged-key-write-1 resolved; LIC-2.6b-hash-key-nil-runtime-1 partial via the new trap surface. 24/0/3 (ADR 0084) |
| ‣ 2.8e-iter-generic generic-for protocol | **Done** | Full Lua 5.4 §3.3.5 `for k, v in ITER, STATE, CTL do BODY end`. Codex Option A (over NaN diagnostic / closure spike). New `StmtKind::ForGeneric` AST variant + `IterMatch::Generic` parser branch + 4 visitor companions in HIR. `lower_stmt(ForGeneric)` synthetic-block desugar (mirrors ADR 0081 ForPairs) pins state / ctl / iter to fresh locals and dispatches the per-iteration call via `Callee::Builtin(Next)` (special `next` ident shortcut), `Callee::User(fid)` (FunctionRef or known-FuncId Local), or `Callee::IndirectDispatch` (TaggedValue local) — closure-as-iter filtered via `f.upvalues.is_empty()` (lifts automatically when ADR 0083 lands). Iter must return `(TaggedValue\|Nil, _)` for nil-termination — Number-only iter rejected. 8 new e2e in `tests/phase2_8e_generic_for.rs` (next-builtin form, user-fn, function-alias, break, nested, immediate-nil termination, closure-reject backstop, Number-only-reject backstop). 951 → 959 green. LIC-2.8e-iter-generic-1 resolved (Phase 1). 25/0/3 (ADR 0085) |
| ‣ 2.6b-hash-key-nan hash key NaN runtime diagnostic | **Done** | Lua spec §3.4.5 forbids NaN as a table index. Codex Option A (over TaggedValue arith coerce / closure spike). New `s_table_index_nan` global ("table index is NaN") + `emit_table_index_nan_trap_if` / `emit_hash_key_nan_preflight` helpers. NaN preflight inserted at 4 sites: static Number-key IndexAssign / Index arms (before `f2i` / bounds-check), inline `emit_local_init_tagged` Number-key arm (covers `print(t[0/0])`), and `emit_hash_probe_loop` entry — single chokepoint for every TaggedValue-key call site, no per-caller duplication. `cmpf Une self-self` reused from ADR 0077's `emit_tonumber_for_arith` (qNaN/sNaN/±NaN agnostic). 6 new e2e in `tests/phase2_6b_hash_key_nan.rs` (static Number write/read traps, TaggedValue NaN via ADR 0074 widening, regression Number / TaggedValue-string / nil-trap). 959 → 965 green. LIC-2.6b-hash-key-nan-runtime-1 resolved. ADR 0087 (2026-05-10) supersedes the `emit_hash_key_nan_preflight` helper portion (folded into the new validity gate); the 3 raw-f64 NaN preflight sites are unaffected. 26/0/2 (ADR 0086) |
| ‣ 2.6b-hash-key-validity hash-key runtime validity policy chokepoint | **Done** | Codex post-3c review v2 (Refactor verdict on plan v2). Pure decision (`enum HashKeyValidityPolicy { TrapNil, CheckNaN }` + `policy_for_tag(tag) -> &'static [...]` in `tagged.rs`) split from effectful executor (`emit_hash_key_runtime_validity_gate` in `emit.rs`). The new gate replaces ADR 0086's `emit_hash_key_nan_preflight` at the `emit_hash_probe_loop` chokepoint (`emit.rs:5535`) and folds in the ADR 0084 inline nil traps at IndexAssign (`emit.rs:3160-3195`) and Index (`emit.rs:6723-6757`) TaggedValue arms — the chokepoint is now the single owner of nil/NaN tag validity for every probe entry (`emit_hash_probe_for_insert`, `emit_hash_probe_lookup`). The 3 raw-f64 NaN preflight sites (`emit.rs:2766` / `:6554` / `:4339`) using `emit_table_index_nan_trap_if` are unaffected — they consume f64 directly, not a tagged slot. Doc-comments on probe wrappers + trap-message globals state the ownership boundary. 3 new pure unit tests in `tagged.rs::tests` (`policy_for_tag` matrix) + 2 new e2e in `tests/phase2_6b_hash_key_nil.rs` (chokepoint trap on TaggedValue-nil insert + read paths). 990 → 995 green. LIC-2.6b-hash-key-nil-runtime-1 resolved (was partial via ADR 0084's inline traps); new pending LIC-2.6b-hash-missing-key-read-1 tracks a separate Lua §3.4.5 violation in the Index TaggedValue arm (uses `emit_hash_probe_lookup` with `trap_on_null=true`, traps on missing key instead of returning nil). 27/0/2 (ADR 0087) |
| ‣ 2.6b-hash-lookup-miss hash read lookup miss reified as Nil-tagged TaggedValue | **Done** | Codex post-0087 review v3 (Refactor verdict on plan v1). New private `enum HashLookupOutcome { NilOnMissing, TrapMissing }` in `emit.rs` (codex critical: lookup miss policy is consumer contract, not tag layer; `tagged.rs` placement was "abstraction without owner"). New chokepoint helper `emit_hash_lookup_into_tagged_slot` consolidates the `null_buf check + for_insert probe + key_at_null check + outcome dispatch` shape duplicated across 9 sites: `emit_local_init_tagged` 4 hash arms (~120 LOC dedupe) + Index 5 hash arms (4 static-key + 1 TaggedValue, restructured to tmp slot + helper(NilOnMissing) + `emit_value_slot_check_number` + load f64). `emit_hash_probe_lookup` wrapper deleted; `trap_on_null: bool` parameter on `emit_hash_probe_loop` retired (codex non-ad-hoc: bool was "粗い abstraction"). User-visible diagnostic shift in arith/cmp contexts: missing key was `s_table_missing_key`, now `s_table_type_mismatch` (consumer-correct). Widening contexts (LocalInit/Assign/print) unchanged via `widen_index_for_local_init` → `IndexTagged` → `emit_local_init_tagged` path. ADR 0084 read-side arms partially superseded; IndexAssign + `pairs`-body idiom (`t[k] = v + 100`) unchanged. 4 new e2e in `tests/phase2_6b_hash_missing_key_read.rs` (2 behaviour-change pins + 2 regression-pins inc. explicit `hash_buf == null` branch coverage). 995 → 999 green. LIC-2.6b-hash-missing-key-read-1 resolved. 28/0/1 (ADR 0088) |
| ‣ 2.7p-tagged-arith-coerce TaggedValue arith operand coercion chokepoint | **Done** | Codex post-0088 review (6 視点 / 6 Go on candidate A: LIC-2.7p-arith-coerce-tagged-1 解放). Pure decision `enum TaggedArithOperandPlan { UseNumberPayload, CoerceStringToNumber, TrapNonNumeric }` + `policy_for_tagged_arith_operand(tag) -> Plan` in `tagged.rs` (mirrors ADR 0087 `policy_for_tag` shape). Effectful chokepoint `emit_load_tagged_operand_as_number` in `emit.rs` recurses over `[TAG_NUMBER, TAG_STRING]` building scf.if dispatch driven by the policy enum, trailing else fires `TrapNonNumeric`. New trap message global `s_arith_on_non_numeric` ("attempt to perform arithmetic on a non-numeric value") for Bool/Nil/Function/Table/Deleted operands; `s_arith_coerce_failed` (ADR 0077) reused for String parse-fail. BinOp dispatcher (`emit_tagged_arith_runtime_dispatch`) covers 12 ops (Add/Sub/Mul/Div/Mod/Pow/FloorDiv + BitAnd/BitOr/BitXor/Shl/Shr); UnaryOp guard covers Neg/BitNot. Eq/Ne / Lt/Le/Gt/Ge / Concat explicitly out of scope per Lua §3.4.4 / existing dispatchers. Mirrors `emit_tagged_eq_runtime_dispatch` (ADR 0066) call-site contract; `tagged_local_idx` extracted to module scope (rule-of-three: Eq + arith). ADR 0077 partially extended (the `emit_tonumber_for_arith` helper is reused drop-in for the runtime path). Existing `arith_on_tagged_local_traps_for_string` test flipped to coerce-success (renamed). 9 new e2e + 3 new unit tests + 2 regression-pins. 999 → 1013 green. LIC-2.7p-arith-coerce-tagged-1 resolved. **Phase 2 tagged-semantics consumer coverage complete** (28/28/0). (ADR 0089) |
| ‣ 2.devinfra-emit CLI pipeline-stage emission `--emit <stage>` | **Done** | Codex post-0089 review v1 → v2 (Refactor verdict on plan v1). New `src/pipeline.rs` use-case module owning `enum EmitStage { Hir, Mlir, Llvm }` + `enum PipelineArtifact { Hir(String), Mlir(String), Llvm(String) }` + `compile_until(source, stage) -> Result<PipelineArtifact>` so CLI / future DAP / LSP / programmatic API reuse the stop-able pipeline (codex critical: pipeline knowledge owner is NOT the I/O adapter). `lumelir compile --emit <stage>` halts at the named stage and writes the artifact's text representation to stdout default (or `-o PATH` to file). Effect boundary explicit: `Hir` / `Mlir` are **render** (pure: Debug fmt / `module.as_operation().to_string()`), `Llvm` is **generate** (effectful: invokes `mlir-opt` + `mlir-translate` subprocesses via existing `codegen::lower::to_llvm_ir`). `src/codegen/` **zero-diff** (CA invariant verified via `git diff --stat`). 5 new e2e in `tests/phase2_devinfra_emit.rs`: 4 stage behaviour with **include + exclude** layer-specific token oracle (per codex critical #3) + 1 regression-pin asserting the no-emit full-compile path is unchanged. 1013 → 1018 green. No LIC change (dev-infra). New cross-cutting `2.devinfra-*` phase tag introduced for non-language-feature work; future container ADR (deferred per ADR 0005) and DAP ADR (roadmap-only — prerequisites: source-location metadata + debug-runtime contract) will reuse the tag. ADR 0005 unchanged. (ADR 0090) |
| ‣ 2.6+-callee-norm HIR callee normalization for Index-callee Calls | **Done** | Plan v2 post-abort (v1 "method colon syntax" aborted 2026-05-11 when HIR implementation surfaced 4 cascading prerequisites starting with `lower_call` rejecting any non-Ident callee with `UnsupportedCall`). Codex post-abort review (2026-05-14) reframed: scope is **HIR callable boundary**, not syntax sugar. Pure classifier `classify_callee_form` (DirectIdent / IndexCallee) + effectful executor `materialize_callee_to_local` (pre-binds Index result to synthetic `__callee_<N>` TaggedValue local via `widen_index_for_local_init`, ADR 0063 storage rule reuse) + new `LowerCtx::pending_pre_stmts` hoisting buffer + `lower_stmt` drain wrapper. `lower_call` entry dispatches via classifier; IndexCallee path pushes pre-bind LocalInit then recurses with synthetic Ident, routing through existing `Callee::IndirectDispatch` (ADR 0082) — LocalId-source invariant preserved (codex critical: no new Callee variant). Infrastructure is general-purpose (future Methods sugar / `__call` metamethod / let-binding rewrites). `src/codegen/`, `src/cli/`, `src/pipeline.rs`, `src/parser/`, `src/lexer/` **zero-diff** (CA invariant). 6 new e2e in `tests/phase2_index_callee.rs` (3 happy-path Red → Green + 1 always-green regression-pin for `local g = t.m; g(args)` + 2 typed-error pins: `IndirectCallNoCandidates` for no-match sig, runtime `s_call_non_function` trap for non-Function callee). 1018 → 1024 green. Methods (`obj:method()` colon syntax) landed in ADR 0092. (ADR 0091) |
| ‣ 2.6+-methods Method colon syntax desugar | **Done** | Codex post-0091 review (6 視点, 4 critical fixes baked in: "no sugar-only framing" / "self kind upfront" / "HIR-chokepoint desugar, not parser-completed" / "receiver-shape check explicit"). New lexer `TokenKind::Colon` + single-char dispatch; new AST `ExprKind::MethodCall { receiver, method, args }` and `StmtKind::MethodDef { receiver, method, is_colon, params, body }` preserving source shape (single-segment Ident receiver only for MVP); parser adds Colon arm to `parse_call_suffix` + `parse_method_def` helper gated by Ident-lookahead so expression-position `function() ... end` keeps flowing through FunctionExpr. HIR chokepoint: `materialize_callee_to_local` renamed `materialize_to_synth_local` accepting any `&Expr` (Tidy-First; one helper, two roles). `lower_expr` MethodCall arm desugars `recv:m(args)` to `Call(Index(recv, Str(m)), [recv, ...args])` then recurses through ADR 0091's IndexCallee path. `lower_method_def` prepends `"self"` to effective_params when `is_colon`, seeds `external_kinds[0] = Table` (MVP — future ADR widens to TaggedValue once dispatcher gains arg widening; chosen because ADR 0082's strict-equal sig matching rejects Table-receiver / TaggedValue-param mismatches), registers anon function via FunctionExpr-style flow, emits IndexAssign(recv, Str(method), FunctionRef). Pure `check_method_receiver_shape` recursive walker rejects `Call/MethodCall/FunctionExpr/BinOp/UnaryOp` as new `HirError::ComplexMethodReceiver`. Visitor arms added to `infer_param_kinds` / `infer_user_function_param_kinds` (descend without refinement extension — same carry-over as ADR 0091). Hetero-return method bodies trip existing LIC-2.6c-tag-locals-fn-indirect-1 via IndexAssign function-value branch. `src/codegen/`, `src/cli/`, `src/pipeline.rs` **zero-diff** (CA invariant). 7 new e2e in `tests/phase2_method_syntax.rs` (4 happy: colon-def-and-call / dotted-def-and-call / multi-arg / dual-form-callable + 1 always-green regression-pin + 2 typed-error pins: ComplexMethodReceiver / bare-top-level-function-rejected). 1024 → 1031 green. MethodCall arg refinement extended in ADR 0093. (ADR 0092) |
| ‣ 2.6+-method-arg-refine MethodCall arg refinement | **Done** | Codex post-0092 review (6 視点) verdict Refactor → Go with critical fix: pass-order — `infer_user_function_param_kinds` runs BEFORE lowering, so MethodDef FuncIds must be pre-allocated in Pass 1 mirroring FunctionDef's `register_function_signature`. New `register_method_signature` helper in `src/hir/mod.rs` produces the same placeholder shape (mangled `user_anon_<idx>`, default Number param kinds) plus inserts `(receiver, method) -> FuncId` into a new `LowerCtx::method_funcs` HashMap. The index is threaded through `LowerCtx::new` / `for_function` / `lower_into_function`. `lower()` Pass 1 walks MethodDef stmts sequentially after FunctionDef so the `funcdef_seq` counter at Pass 2 still maps 1:1 onto FunctionDef FuncIds. `infer_user_function_param_kinds` signature extended with `method_funcs`; MethodCall arm rewrites from ADR 0092's descend-only to refinement-extended (Ident receiver required for static FuncId resolution; args index 1..N refined from literal kinds via `ast_arg_kind`; `seen[idx]` first-call-site-wins matches FunctionDef). `lower_method_def` switches from inline FuncId alloc to `method_funcs` lookup; `external_kinds` reads `functions[id.0].params` (carries Pass-1.5 refinement) with self at index 0 re-seeded to Table per ADR 0092 policy. `#[allow(clippy::too_many_arguments)]` added to `for_function` (8 args after plumbing; internal helper, single-source). `src/codegen/`, `src/cli/`, `src/pipeline.rs`, `src/parser/`, `src/lexer/` **zero-diff** (CA invariant). 4 new e2e in `tests/phase2_method_arg_refine.rs` (3 happy Red → Green: colon String arg / colon Bool arg / colon multi-String args + 1 always-green regression-pin asserting FunctionDef + Ident-Call refinement path unchanged). 1031 → 1035 green. Index-callee Call refinement closed in ADR 0094. (ADR 0093) |
| ‣ 2.6+-method-idx-call-refine Index-callee Call arg refinement | **Done** | Codex post-0093 review (6 視点) verdict Refactor → Go with critical: extract shared kinds/seen update helper so three refinement arms (Ident-Call / MethodCall / Index-callee Call) don't duplicate. New `try_refine_func_args(idx, base, args, kinds, seen)` pure helper nested in `infer_user_function_param_kinds`. Existing Ident-Call arm refactored to use helper with `base=0`; existing MethodCall arm with `base=1`. New Index-callee refinement: secondary if-let inside the `Call` arm matching `callee = Index { target: Ident, key: Str }` and looking up `(target_name, key_str)` in `method_funcs` (ADR 0093 reuse) — uses `base=0` because Index-callee is the explicit-self / dotted-call form with no implicit self injection. Non-Ident target / non-Str key safely skips via lookup miss. For colon-def + explicit-self call `t.m(t, x)`, the kinds[idx][0]=Table refinement from `t` is a no-op because `lower_method_def` re-seeds external_kinds[0]=Table per ADR 0092 policy at the for_function call site (documented as no-op intersection). `src/codegen/`, `src/cli/`, `src/pipeline.rs`, `src/parser/`, `src/lexer/` **zero-diff** (CA invariant). 3 new e2e in `tests/phase2_method_idx_call_refine.rs` (2 happy Red → Green: dotted-def Index-callee String arg / colon-def explicit-self String arg + 1 always-green regression-pin asserting ADR 0093 MethodCall path unchanged after the helper extract refactor). 1035 → 1038 green. Multi-segment method-def carry-over closed via ADR 0095. (ADR 0094) |
| ‣ 2.6+-nested-index-assign-widen Nested IndexAssign / Index target widening | **Done** | Codex review for multi-segment method-def returned Refactor → Go; pre-implementation exploration surfaced deeper prereq (`app.utils.field = 10` already failed at HIR target_kind check); user steered non-ad-hoc via AskUserQuestion → pivoted to chokepoint fix. New `widen_index_for_assign_target` HIR helper (mirrors ADR 0063 `widen_index_for_local_init` idempotent shape) rewrites `HirExprKind::Index` → `IndexTagged` at IndexAssign and Index target positions. target_kind check at both sites loosened to accept TaggedValue in addition to Table. Codegen: new `emit_resolve_table_target_ptr` dispatch helper (single chokepoint reused by Index read / IndexAssign write / `emit_local_init_tagged` source) routes IndexTagged targets through `emit_narrow_indextagged_to_table_ptr` — alloca tmp tagged slot, run existing `emit_local_init_tagged`, check tag == TAG_TABLE, trap with new `s_index_target_not_table` global (Lua spec §3.4.11) on mismatch, extract Table descriptor as `!llvm.ptr` via `llvm.inttoptr`. Idempotent on non-Index targets — single-level IndexAssign path (ADR 0055) unchanged. CA invariant deviation documented; `src/codegen/` ~175 LOC delta bounded to `emit.rs` (helper extract + narrowing chokepoint + 3 call-site swaps + trap global). `src/parser/`, `src/lexer/`, `src/cli/`, `src/pipeline.rs` **zero-diff**. 4 new e2e in `tests/phase2_nested_index_assign.rs` (3 happy Red → Green: nested field write+read / nested Number-key write+read / write-twice overwrite + 1 always-green regression-pin asserting single-level IndexAssign path unchanged). 1038 → 1042 green. Multi-segment method-def parser delta closed via ADR 0096. (ADR 0095) |
| ‣ 2.6+-multi-segment-method-def Multi-segment method-def parser delta | **Done** | Codex post-0095 review (6 視点) verdict Refactor → Go with critical: FuncId allocation must happen for ALL MethodDef regardless of segment count, `method_funcs` index limitation only governs call-site refinement. AST: `StmtKind::MethodDef.receiver: String` renamed to `receiver_chain: Vec<String>` (length-1 = ADR 0092 single-segment path). Parser `parse_method_def` loops over `.IDENT` segments and terminates at `:IDENT` (colon-form) or LParen (dotted-form, last segment is method); bare-top-level `function NAME()` (segments.len() < 2 after loop) still rejects with `UnexpectedToken { LParen }` matching ADR 0092 pin. HIR: `register_method_signature` split into alloc-only `alloc_method_signature` (always allocates FuncId + pushes HirFunction placeholder) + caller-side conditional `method_funcs` insertion (gated to `receiver_chain.len() == 1` for call-site refinement boundary; ADR 0097 lifts this gate). New `LowerCtx::methoddef_func_ids: Vec<FuncId>` + `methoddef_seq: usize` threaded through `new` / `for_function` / `lower_into_function`; mirrors `funcdef_seq` pattern (proven). `lower_method_def` folds `receiver_chain` into nested `Expr::Ident → Expr::Index` chain, lowers via `lower_expr` + applies ADR 0095 `widen_index_for_assign_target` (idempotent for length-1; nested target widens to TaggedValue for length ≥ 2 → codegen TAG_TABLE narrow). target_kind check loosened to accept TaggedValue (ADR 0095 sibling). `src/codegen/`, `src/cli/`, `src/pipeline.rs`, `src/lexer/` **zero-diff** (CA invariant). 4 new e2e in `tests/phase2_multi_segment_method_def.rs` (3 happy Red → Green: 3-segment dotted-def Number arg / 3-segment colon-def compile-only / 4-segment boundary + 1 always-green regression-pin asserting ADR 0092 2-segment path unchanged). 1042 → 1046 green. Call-site refinement for multi-segment closed in ADR 0097. (ADR 0096) |
| ‣ 2.6+-multi-seg-call-refine Multi-segment method-call refinement | **Done** | Codex post-0096 review (6 視点) verdict Refactor → Go with critical: unify `method_funcs` to chain-keyed `HashMap<(Vec<String>, String), FuncId>` — single-segment uses length-1 chain key, don't maintain two indices. Pass-1 walk drops the `receiver_chain.len() == 1` gate (introduced by ADR 0096); ALL MethodDef now enter `method_funcs` keyed by their full chain. New pure walker `extract_index_chain(callee: &Expr) -> Option<(Vec<String>, String)>` recursively walks `Index{Index{...{Ident, Str}...}, Str}` chains, builds chain head-first, returns method as outermost key; non-Ident head or non-Str key → None (safe skip). `infer_user_function_param_kinds` Call arm rewired: existing single-segment if-let REPLACED by `extract_index_chain` + chain-keyed lookup → `try_refine_func_args(idx, 0, ...)` (ADR 0094 helper reuse). MethodCall arm gets length-1 wrap for the single-Ident receiver path. `src/codegen/`, `src/cli/`, `src/pipeline.rs`, `src/parser/`, `src/lexer/` **zero-diff** (HIR-only refinement). 3 new e2e in `tests/phase2_multi_seg_call_refine.rs` (2 happy Red → Green: 3-seg dotted call String arg / 4-seg dotted call String arg + 1 always-green regression-pin asserting single-segment refinement path unchanged after the chain-key unification). Closes ADR 0091/0094/0096 collective carry-over for dotted multi-segment call path (e.g. `app.utils.format("world")` now refines `name` to String → dispatch matches end-to-end). 1046 → 1049 green. Name-rebind refinement closed in ADR 0098. (ADR 0097) |
| ‣ 2.6+-name-rebind-refine Top-level name-rebind refinement | **Done** | Codex post-0097 review (6 視点) verdict Refactor → Go with critical: use Pass-1.5 pure `alias_map`, NOT extend `LocalInfo.func_id` (would mix pre-pass refinement facts with post-lowering metadata). Closes ADR 0097 future-work for `local g = a.b.method; g(arg)` rebind pattern. New `alias_map: HashMap<String, FuncId>` built in Pass-1 walk over chunk top-level `StmtKind::Local` / `LocalMulti` (after `method_funcs` builds). For each binding, `extract_index_chain` (ADR 0097 reuse) resolves the RHS shape; on `method_funcs[(chain, method)]` hit, `(name, FuncId)` inserts. Last-wins rebind shadowing (HashMap insert, same as `function_names` / `method_funcs` carry-over). `infer_user_function_param_kinds` extended with `alias_map` parameter; Call arm: after `function_names` lookup, ALSO try `alias_map[name]` when callee is `Ident` and not in `function_names`, refine via `try_refine_func_args(idx, 0, ...)` (ADR 0094 helper reuse). Lookup priority: function_names > alias_map > method_funcs (chain-keyed). `src/codegen/`, `src/cli/`, `src/pipeline.rs`, `src/parser/`, `src/lexer/` **zero-diff** (HIR-only). 4 new e2e in `tests/phase2_name_rebind_refine.rs`: 2 happy Red → Green (single-seg rebind String / multi-seg rebind String) + 1 always-green regression-pin (no-rebind path unchanged) + 1 codex-critical negative pin `shadowed_rebind_uses_last_def` (last-wins refinement targeting via two `local g = ...` shadows). 1049 → 1053 green. Multi-step alias chains closed via ADR 0099. (ADR 0098) |
| ‣ 2.6+-multi-step-alias Top-level multi-step alias chain | **Done** | Codex post-0098 review (6 視点) verdict Refactor → Go with critical: incorporate fixed-point into ADR 0098 build phase NOT a separate Call-side helper; insert-only monotonic guarantees termination. Closes ADR 0098 future-work for `local h = a.b.m; local g = h; g(x)` multi-step Ident → Ident rebinding. Pass-1 `alias_map` build extended with Round 2+ fixed-point closure: after Round 1 (Index-chain via `extract_index_chain`), iterate chunk top-level `StmtKind::Local` / `LocalMulti` whose RHS is bare `ExprKind::Ident(other)`; if `alias_map[other]` exists AND `!alias_map.contains_key(name)`, insert `(name, alias_map[other])` and mark `changed`. Loop terminates when no insert happens in a full pass. Insert-only guards termination: each iteration strictly grows `alias_map` over a finite set of top-level Local names (worst-case O(N²), in practice 2-3 iterations). Round 1's last-wins shadowing preserved (ADR 0098 backward-compat); Round 2's insert-only is the documented divergence for rebind-of-rebind. ADR 0098's Call arm logic unchanged (lookup priority function_names > alias_map > method_funcs). Lua scoping forbids forward-reference so cycles cannot form at chunk level. `src/codegen/`, `src/cli/`, `src/pipeline.rs`, `src/parser/`, `src/lexer/` **zero-diff** (HIR-only ~20 LOC extension). 3 new e2e in `tests/phase2_multi_step_alias.rs`: 2 happy Red → Green (2-step `local h = ...; local g = h; g(arg)` / 3-step `local i = ...; local h = i; local g = h; g(arg)`) + 1 always-green codex-critical regression-pin asserting ADR 0098 single-step path unchanged after the fixed-point extension. 1053 → 1056 green. Function-body rebind, re-assignment alias, block-scoped scope tracking, method-call rebind, aliasing chains crossing function_names spaces remain future work. (ADR 0099) |
| ‣ 2.6+ tables / metatables | In progress | Methods (`obj:method()`) landed in ADR 0092; MethodCall arg refinement closed via Pass-1 MethodDef registration in ADR 0093; Index-callee Call arg refinement + helper extract closed in ADR 0094; nested IndexAssign / Index target widening closed via TAG_TABLE runtime narrow in ADR 0095 (enables `app.utils.field = 10`); multi-segment method-def parser delta closed in ADR 0096 (`function a.b.c.m() end` / `function a.b.c:m() end`); multi-segment call-site refinement closed in ADR 0097 (`app.utils.format("world")` end-to-end); name-rebind refinement closed in ADR 0098 (`local g = a.b.format; g("world")`); multi-step alias chains closed in ADR 0099 (`local h = a.b.m; local g = h; g("world")`). Function-kind upvalue support (lifts MutualCapturingRecursion + IndirectCallNonNumberReturn backstops), multi-segment colon-call (MethodCall with Index receiver), receiver kind narrowing, function-body alias / re-assignment alias / method-call rebind, metatables — future ADRs. Phase 2 tagged-semantics consumer coverage complete as of ADR 0089. |
| Phase 3 — Domain Features | Not started | Rust-Lua inline bridge, embedded register dialect |

**How to read TBD markers:** sections marked `TBD: Phase N, ADR XXXX` indicate the rule is undecided until that ADR lands. Do not invent answers — surface the question instead.

## 3. Required Reading Before You Start

1. [`docs/PRD.jp.md`](docs/PRD.jp.md) — product intent (SoT)
2. [`docs/design/README.md`](docs/design/README.md) — ADR conventions
3. `docs/design/NNNN-*.md` — any ADRs relevant to your task
4. This file
5. Existing tests of the module you're touching

**Phase 2.6c (TaggedValue) work:** also read
[`docs/design/tagged-semantics.md`](docs/design/tagged-semantics.md)
— the Single Source of Truth for slot layout, producer / consumer
matrix, runtime invariants, and the consolidated LIC table (ADR 0068).

## 4. Coding Principles

### 4.1 Functional Programming First

- **Pure functions by default.** Keep data flow as `input → pure transform → output`.
- Push side effects (file I/O, stdout, process spawn, allocator choice) to layer boundaries.
- Prefer `Iterator` adapters and `map`/`fold` over mutable accumulators.
- **Escape hatch:** impurity is permitted when profiling shows it matters (e.g. tokenizer buffer reuse). Justify with a comment *and* an ADR if the API leaks mutation.
- Examples:
  - Preferred: `fn tokenize(src: &str) -> Result<Vec<Token>, LexError>`
  - Justify-in-ADR: `fn tokenize(&mut self, src: &str)` (internal buffer reuse)

### 4.2 Clean Architecture (Layering)

Dependency direction (outer → inner):

```
cli  →  (lib crate root, Phase 1+)  →  codegen  →  mir  →  hir  →  parser  →  lexer
```

- Each layer may only `use` items from layers **strictly inside it**. Reverse dependencies are forbidden.
- MLIR / Melior / LLVM-sys bindings are confined to the `codegen` layer. `hir` / `mir` use plain Rust types.
- Phase 1 adopted layering: `src/lib.rs` as the library root, `src/main.rs` as a thin entry (<20 lines) calling `lumelir::cli::run()`. See [ADR 0002](docs/design/0002-lib-rs-layering.md).

### 4.3 Test-Driven Development

Cycle: **Red → Green → Refactor.**

1. **Red** — write a failing test first. Scope it: `cargo test --lib lexer::tests::lex_integer`.
2. **Green** — write the minimum code to pass. Ugly is fine.
3. **Refactor** — keep tests green while improving structure.

Commit granularity: one commit per red→green transition is ideal but not enforced; refactor commits stay separate.

Test placement:
- **Unit** (pure logic): at the end of the module file, inside `#[cfg(test)] mod tests { ... }`.
- **Integration** (CLI, file I/O): under `tests/` (e.g. `tests/cli_compile.rs`).
- **Fixtures**: `tests/fixtures/*.lua`.

Test naming convention: `fn <subject>_<condition>_<expectation>()`. Example: `fn lex_integer_literal_yields_single_number_token()`.

### 4.4 Rust-Specific Guidance

- Lint gate: `cargo clippy --all-targets -- -D warnings` must pass.
- `unwrap` / `expect` are **forbidden in non-test code** unless justified with a comment explaining why the invariant holds.
- Error types: library layers use `thiserror`-derived enums; the CLI layer may use `anyhow` to collapse them at the boundary. See [ADR 0003](docs/design/0003-error-handling.md).
- `unsafe` requires a `// SAFETY:` comment and is confined to MLIR/LLVM FFI boundaries.
- Avoid `Clone` unless there is a clear ownership reason; prefer borrowing.
- No premature abstractions. Follow the "rule of three" before extracting a helper.

## 5. Test Conventions (Summary)

- Run locally: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`.
- CI: **TBD: Phase 1** (GitHub Actions expected). When added, the above command sequence is the minimum gate.
- Property-based / fuzz testing: **TBD: Phase 2+** (not yet warranted).

## 6. Commits & Pull Requests

### 6.1 Conventional Commits

Format: `<type>(<optional-scope>): <subject>`

Allowed types: `feat`, `fix`, `chore`, `docs`, `test`, `refactor`, `perf`, `build`, `ci`.

Subject rules: imperative mood, lowercase start, no trailing period, ≤72 chars.

Examples from this repo's history:
- `chore: initial scaffold for LuMeLIR (Rust 2024 edition)`
- `chore: track .claude/.gitignore to share local-settings exclusion rule`

### 6.2 PR Discipline

- One PR = one logical change. If you find yourself writing "and" in the PR title, split it.
- Link the relevant ADR number in the PR description when a design decision is involved.
- A PR that changes behavior without tests is **not mergeable**.

## 7. ADR Workflow

Conventions live in [`docs/design/README.md`](docs/design/README.md). Recap:

- Filename: `NNNN-kebab-title.md` (zero-padded, monotonic).
- Write an ADR when:
  - adding a new crate dependency,
  - changing module/layer boundaries,
  - making a deliberate trade-off between performance and readability/maintainability,
  - choosing between two viable implementation strategies.
- Reference the ADR number in the PR description and in commit messages where helpful.

## 8. Dependency Addition Policy

- **Do not add crates for phases that have not started.** The Phase 0 rule (`clap` only until Phase 1 begins) generalizes: add dependencies at the moment they are first needed, together with an ADR.
- When adding a dependency:
  1. Justify in an ADR (alternatives considered, trade-offs).
  2. Use it in the same PR that adds it — no placeholder additions.
  3. Check `cargo tree` for unexpected transitive dependencies.
- Phase 1 expected additions (gated on ADRs): lexer crate (if not hand-rolled), `melior`, `thiserror`/`anyhow`.

## 9. Documentation Update Policy

- **[`docs/PRD.jp.md`](docs/PRD.jp.md) is the Source of Truth.** [`docs/PRD.md`](docs/PRD.md) is a best-effort English translation and may drift — keep the footer pointing back to the Japanese SoT.
- [`README.md`](README.md) (English) is primary; [`docs/README.jp.md`](docs/README.jp.md) is the translation.
- **When you change a policy in this file, update it in the same commit as the code/ADR change.** Stale AGENTS.md is the worst failure mode.

### 9.1 TaggedValue SoT update checklist

[`docs/design/tagged-semantics.md`](docs/design/tagged-semantics.md) is the
SoT for the Phase 2.6c TaggedValue runtime model (ADR 0068). When a PR
touches:

- `src/codegen/emit.rs` TaggedValue dispatch helpers
  (`emit_value_slot_*`, `emit_local_init_tagged`, `emit_isnil_index`,
  `emit_print_tagged_local`, `emit_type_tagged_local`,
  `emit_tostring_tagged_local`, `emit_tagged_eq_*`,
  `emit_tagged_unknown_tag_trap`)
- `src/hir/mod.rs` HIR variants for tagged values
  (`HirExprKind::IndexTagged`, `IsNil`, `ValueKind::TaggedValue`)
- Any test under `tests/phase2_6c_tag_*`

… confirm the SoT doc is up to date. The ADR's *Documentation updates*
checklist (per `docs/design/README.md` template) records which sections
were touched, or justifies "no change required". Stale `tagged-semantics.md`
is the second-worst failure mode after stale `AGENTS.md`.

## 10. LLM-Agent-Specific Rules

### 10.1 Destructive Operations Require Explicit Human Approval

Do **not** run the following without the user explicitly asking:

- `git reset --hard`, `git push --force`, `git branch -D`, `git checkout -- .`, `git clean -fd`, `git rebase -i`
- `rm -rf`, recursive directory moves
- `cargo clean` (usually fine but confirm first)
- Any operation that rewrites published history

### 10.2 Do Not Touch

- `.claude/settings.local.json` — user-local Claude Code settings, excluded via `.claude/.gitignore`.
- `git config` — both repository and global scope are off-limits.
- `LICENSE-APACHE`, `LICENSE-MIT` — licensing text is fixed.
- `Cargo.lock` — do not hand-edit. Let `cargo` regenerate it.

### 10.3 Environment Gotchas

**Primary dev environment: WSL2 Arch Linux (see [ADR 0005](docs/design/0005-mlir-environment.md)).** Working tree lives at `~/LuMeLIR` (native ext4). Anything that pulls `melior` / `mlir-sys` needs WSL2; pure-Rust layers also build on Windows but Windows native MLIR is best-effort only.

Under WSL2 Arch Linux:
- MLIR toolchain: `sudo pacman -S base-devel llvm rust cmake ninja pkgconf clang zlib zstd libxml2` plus `paru -S mlir` (AUR; matches melior 0.27 = MLIR 22).
- Env vars for `melior` (put in `~/.bashrc` or repo-local script):
  ```bash
  export MLIR_SYS_220_PREFIX=/usr
  export LLVM_SYS_220_PREFIX=/usr
  export TABLEGEN_220_PREFIX=/usr
  ```
- Sanity check: `llvm-config --version` and `mlir-tblgen --version` should both report 22.x.
- `LIBCLANG_PATH` from a Windows host (e.g. Xtensa ESP clang) is sometimes imported into WSL2 shells. Usually harmless, but if `bindgen` complains about a weird libclang, `unset LIBCLANG_PATH` first.

Historical Windows + Git Bash notes (kept for pure-Rust layers; do not run MLIR-linked builds here):
- Shell is Git Bash / MSYS2-style. Use Unix syntax: `/dev/null` not `NUL`.
- `/usr/bin/link.exe` (Git Bash) shadows MSVC `link.exe`. WSL2 sidesteps this.
- Out-of-tree `/mnt/v/melior-spike/` records the Windows MSVC port attempt; see its `FINDINGS.md` before re-trying.

### 10.4 Commits & Pushes Require Explicit Instruction

- Never commit autonomously. Wait for the user to say "commit this" or equivalent.
- Never push without explicit instruction.
- Format commit messages per §6.1.

### 10.5 When in Doubt, Ask

If the task is ambiguous, ask the user before writing code. Blindly guessing at intent produces work that gets thrown away and wastes context. A short question beats a long wrong implementation.

## 11. TBD — Decisions Pending

Replace each entry with an ADR link once the decision lands.

- **CI configuration**: GitHub Actions workflow for fmt / clippy / test / (future) cross-compile
- **`.gitattributes` / `rustfmt.toml`**: formal line-ending and formatting rules
- **MLIR dialect ownership**: which layer owns FFI, dialect registration, first op set for Phase 1 (ADR pending once the first real codegen lands under WSL2)
- **Windows native MLIR support**: re-opening after ADR 0005 — tracked out-of-tree in `V:/melior-spike/FINDINGS.md`; returns as a future ADR once upstream tblgen accepts the patches

### Resolved
- Lexer implementation → [ADR 0001](docs/design/0001-lexer-implementation.md) (hand-written)
- Library/binary split → [ADR 0002](docs/design/0002-lib-rs-layering.md) (`lib.rs` + thin `main.rs`)
- Error handling → [ADR 0003](docs/design/0003-error-handling.md) (`thiserror` / `anyhow` boundary)
- Parser implementation → [ADR 0004](docs/design/0004-parser-implementation.md) (recursive descent + Pratt)
- MLIR integration environment → [ADR 0005](docs/design/0005-mlir-environment.md) (WSL2 Arch primary, Windows native best-effort)

## 12. References

- [`README.md`](README.md) — English overview
- [`docs/README.jp.md`](docs/README.jp.md) — Japanese overview
- [`docs/PRD.jp.md`](docs/PRD.jp.md) — Product Requirements (SoT)
- [`docs/PRD.md`](docs/PRD.md) — Product Requirements (EN translation)
- [`docs/design/README.md`](docs/design/README.md) — ADR conventions and index
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — Human contributor guide
- [`CLAUDE.md`](CLAUDE.md) — Pointer for Claude Code
