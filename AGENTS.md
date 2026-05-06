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
| ‣ 2.5c-full full closures (Plan B) | **Accepted, deferred** | ADR 0083 plan landed via Codex pre-review (closure object ptr 一本化 + shared upvalue boxes); MLIR feasibility for `llvm.mlir.global` with computed initializer body referencing `func.func` symbols via `llvm.mlir.addressof` is unverified, so commit-by-commit landing is multi-session work. Commit 1 in `e6b256f` ships `src/codegen/closure.rs` skeleton (layout constants + 8 helper stubs). Remaining commits (TAG_FUNCTION semantic cutover, captured-local boxes, escape relax, generic-for unlock) deferred to a future session per Codex review §1 — `static singleton emission` is fine to land separately, `payload semantic cutover` must be atomic across producer+consumer (944 tests at risk during partial state) (ADR 0083) |
| ‣ 2.8e-iter-tk TaggedValue-key IndexAssign + Index read | **Done** | Codex (C) pivot — ADR 0083 deferred, ADR 0084 先行. HIR `is_hash_key_eligible` accepts `ValueKind::TaggedValue` (1-line relax). Codegen runtime tag dispatch in IndexAssign / Index: tag check first (`TAG_NIL` trap via new `s_table_index_nil`, Lua spec §3.4.5), pin the local's slot as the search-key slot directly (no fresh `emit_build_search_key_slot` tmp), hash probe via the existing ADR 0079 dispatched helpers. New-key commit copies the 16-byte search slot into `entry+0` raw (no kind-aware store needed). Resolves the natural `for k, v in pairs(t) do t[k] = v + 100 end` idiom; ADR 0080's `pairs_body_writes_separate_table_safely` workaround reframed to `pairs_body_mutates_existing_value_safely`. 7 new e2e in `tests/phase2_8e_tagged_key_indexassign.rs`. Array path bypassed for TaggedValue keys (documented limitation: Number-keyed reads still see array slot, not hash mirror). 944 → 951 green. LIC-2.8e-pairs-tagged-key-write-1 resolved; LIC-2.6b-hash-key-nil-runtime-1 partial via the new trap surface. 24/0/3 (ADR 0084) |
| ‣ 2.8e-iter-generic generic-for protocol | **Done** | Full Lua 5.4 §3.3.5 `for k, v in ITER, STATE, CTL do BODY end`. Codex Option A (over NaN diagnostic / closure spike). New `StmtKind::ForGeneric` AST variant + `IterMatch::Generic` parser branch + 4 visitor companions in HIR. `lower_stmt(ForGeneric)` synthetic-block desugar (mirrors ADR 0081 ForPairs) pins state / ctl / iter to fresh locals and dispatches the per-iteration call via `Callee::Builtin(Next)` (special `next` ident shortcut), `Callee::User(fid)` (FunctionRef or known-FuncId Local), or `Callee::IndirectDispatch` (TaggedValue local) — closure-as-iter filtered via `f.upvalues.is_empty()` (lifts automatically when ADR 0083 lands). Iter must return `(TaggedValue\|Nil, _)` for nil-termination — Number-only iter rejected. 8 new e2e in `tests/phase2_8e_generic_for.rs` (next-builtin form, user-fn, function-alias, break, nested, immediate-nil termination, closure-reject backstop, Number-only-reject backstop). 951 → 959 green. LIC-2.8e-iter-generic-1 resolved (Phase 1). 25/0/3 (ADR 0085) |
| ‣ 2.6+ tables / metatables | In progress | full closures (ADR 0083 deferred), closure-with-upvalues, runtime hash-key NaN diagnostic, methods, metatables — multiple sub-phases |
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
