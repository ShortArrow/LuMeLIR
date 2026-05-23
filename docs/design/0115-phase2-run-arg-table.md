# 0115. Phase 2.8f-run-arg-table: `arg` table for script CLI args

- **Status:** Accepted
- **Kind:** Feature Memo
- **Date:** 2026-05-22 (commit `b053774`)
- **Deciders:** ShortArrow

## Replan provenance

Recent CLI work (`a6c9e8d` + `93fb290`) extended `lumelir run`
to accept file path / inline code / explicit `-` stdin /
implicit-pipe stdin — the **input** surface. The natural next
step is the **runtime** surface: passing CLI args to the script.

Codex post-CLI 6-視点 verdict **D (= A `arg` + B `io.write`)
= Strong Go**, implemented A first per the agreed A→B sequence
(ADR 0116 follows for io.write).

## Codex critical fixes baked in

1. **No Phase 3 globals** — `arg` is **synthesised at the CLI
   boundary** as a `local arg = {...}` AST stmt prepended to
   the parsed chunk, not by introducing a globals concept.
2. **`compile_until` pure boundary unchanged** — `arg` plumbing
   lives in `src/cli/run.rs`; `src/pipeline.rs` zero-diff.
3. **Lua scope rules respected** — user `local arg = {...}` after
   the synthetic prelude shadows it (test pin).

## Non-goals (top-of-ADR)

- **`arg[0]` (script name) / negative indices** — Lua §8.1 full
  spec; MVP scope is `arg[1]+` positional only. Future ADR.
- **Phase 3 globals** — `arg` is a local, not a global.
- **TaggedValue runtime tag dispatch for `arg[N]`-typed values
  in arith/compare contexts** — HIR static-typing limitation
  carried over (pre-existing).
- **Dynamic `arg` mutation by the user** — `local arg = {99}`
  rebinds; no globals/upvalues required.

## Goals

1. `lumelir run [INPUT] [SCRIPT_ARGS...]` accepts trailing
   positional args.
2. Synthetic AST prelude `local arg = {"arg0", "arg1", ...}` is
   prepended to the parsed chunk.
3. HIR / codegen / parser / lexer **zero-diff**.
4. All 4 input modes (file / inline / explicit-stdin / implicit-
   pipe) pass `arg` correctly.
5. Test corpus: 1204 → 1216 (+12).

## Lua 5.4 §6.1 / §8.1 compliance

- `arg` is the table holding script CLI args.
- Reference impl: `arg[0]` = script name, `arg[1]+` = positional
  args, negative index = command-line-interpreter args.
- MVP deviation: only `arg[1]+`. `arg[0]` and negative indices
  are nil. Documented limitation.

## 設計

### CLI (`src/cli/mod.rs`)

```rust
Run {
    input: Option<String>,
    /// Trailing positional args passed to the Lua script as
    /// `arg[1]`, `arg[2]`, ... (Lua §6.1 / §8.1 MVP scope).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    script_args: Vec<String>,
},
```

`trailing_var_arg = true` makes everything after the (optional)
input positional flow into `script_args`. `allow_hyphen_values
= true` lets `lumelir run - a b` parse with INPUT=`-` and
SCRIPT_ARGS=["a", "b"] without clap mistaking the second `-` for
a flag.

### Synthetic prelude (`src/cli/run.rs::inject_arg_table_prelude`)

```rust
fn inject_arg_table_prelude(chunk: &mut Chunk, script_args: &[String]) {
    let zero_span = Span { start: 0, end: 0 };
    let entries: Vec<Expr> = script_args
        .iter()
        .map(|s| Expr::new(ExprKind::Str(s.clone()), zero_span))
        .collect();
    let table = Expr::new(ExprKind::Table(entries), zero_span);
    let arg_stmt = Stmt::new(
        StmtKind::Local {
            name: "arg".to_owned(),
            value: table,
        },
        zero_span,
    );
    chunk.insert(0, arg_stmt);
}
```

Call order in `invoke`:

1. `resolve_input` → `(source, label)`
2. `parser::parse(&source)` → `chunk: Chunk`
3. `inject_arg_table_prelude(&mut chunk, script_args)`
4. `hir::lower(&chunk)` (unchanged)
5. `codegen::compile` + execute (unchanged)

AST nodes use `Span { start: 0, end: 0 }` since they have no
source origin; user-source spans drive diagnostics so the
synthetic prelude never emits errors of its own.

## Reuse

| Helper | Path | Purpose |
|---|---|---|
| `Chunk = Vec<Stmt>` | `src/parser/ast.rs:297` | type alias |
| `StmtKind::Local { name, value }` | `src/parser/ast.rs:142-145` | local declaration |
| `ExprKind::Table(Vec<Expr>)` | `src/parser/ast.rs:49` | table constructor |
| `ExprKind::Str(String)` | `src/parser/ast.rs:22` | string literal |
| `Span { start, end }` | `src/lexer/token.rs:6-9` | source span |
| `parser::parse` | `src/parser/mod.rs:18` | parser entry |
| `hir::lower` | `src/hir/mod.rs:1234` | HIR lowering |
| clap `trailing_var_arg` | clap 4.x | variadic positional |

## Codex 6-視点 checklist

- [x] **#1 non-ad-hoc / Tidy First**: synthetic AST prepend keeps
  `compile_until` pure boundary unchanged; no Phase 3 globals
  introduced.
- [x] **#2 TDD**: 12 e2e — count / index / 0 args / empty-string /
  space / oob / file mode / `-` stdin mode / implicit-pipe
  documentation / type(arg) / type(arg[1]) / user-shadow.
- [x] **#3 FP**: pure AST manipulation; the only effect is
  reading CLI args (already effectful) and prepending a
  `Stmt::Local`.
- [x] **#4 CA**: `src/parser/`, `src/lexer/`, `src/hir/`,
  `src/codegen/`, `src/pipeline.rs` **zero-diff**. CLI only.
- [x] **#5 Security**: user-provided args are passed as `String`
  payload of `ExprKind::Str` — never re-lexed, no escape /
  injection surface. Attack surface unchanged.
- [x] **#6 Documentation**: phase tag `2.8f-run-arg-table` is a
  new CLI surface lane (`2.8f-cli-*` group, distinct from
  `2.devinfra-*` dev-infra and `2.7*` stdlib).

## Test count delta

1204 → 1216 (+12) in `tests/phase2_devinfra_run_modes.rs`.

| Test | Category |
|---|---|
| `run_inline_arg_count` | happy |
| `run_inline_arg_index_one_based` | happy |
| `run_inline_arg_zero_extra_args` | edge |
| `run_inline_arg_empty_string_value` | edge (via `print` stdout assertion since HIR rejects `#arg[1]`) |
| `run_inline_arg_with_space` | edge |
| `run_inline_arg_oob_is_nil` | edge |
| `run_file_passes_through_arg_table` | happy |
| `run_explicit_dash_passes_through_arg_table` | happy |
| `run_implicit_pipe_passes_through_arg_table` | documentation (kept as explicit `-` for unambiguous parsing) |
| `run_arg_type_is_table` | static |
| `run_arg_element_type_is_string` | static |
| `run_arg_local_shadow_takes_precedence` | semantics |

Pre-existing HIR limitation surfaces: `arg[1]` is statically
TaggedValue (table-element kind doesn't flow through indexing),
so `#arg[1]` / `arg[1] == ""` are rejected statically; tests
use `print(arg[1])` runtime dispatch instead.

## Critical files

- `src/cli/mod.rs` — `Run.script_args: Vec<String>` field
  (~13 LOC)
- `src/cli/run.rs` — `invoke` signature + `inject_arg_table_prelude`
  (~30 LOC delta)
- `tests/phase2_devinfra_run_modes.rs` — 12 new e2e (~170 LOC)
- `AGENTS.md` — new `‣ 2.8f-run-arg-table` row

**Zero-diff (CA invariant)**:
`src/parser/`, `src/lexer/`, `src/hir/`, `src/codegen/`,
`src/pipeline.rs`.

## Risks

| Risk | Mitigation |
|---|---|
| clap parses `lumelir run - a b` ambiguously | `trailing_var_arg = true` + `allow_hyphen_values = true` + test pin |
| User `local arg = {99}` shadow | Lua scope rule (later `local` shadows earlier); test pin |
| `Span { 0, 0 }` synthetic node confuses diagnostics | Synthetic node never errors; user-source spans untouched |
| `arg[0]` expectation by users | Documented MVP scope; future ADR can add |

## Future work

- **`arg[0]` (script name) + negative indices** — Lua §8.1 full
  spec.
- **TaggedValue runtime tag dispatch for `arg[N]` in arith/compare
  contexts** — broader HIR-static-typing limitation, separate
  ADR.
- **`arg` access in REPL** — when Phase 3 globals land.

## Phase tag

`2.8f-run-arg-table` (CLI surface lane; distinct from
`2.devinfra-*` dev-infra and `2.7*` stdlib lanes).
