# 0315. N5-A coroutine runtime foundation — `coroutine.create` (design)

- **Status:** Accepted (design; implementation is the N5-A arc)
- **Kind:** Architecture Decision
- **Date:** 2026-07-14
- **Deciders:** ShortArrow

## Context

N5 (roadmap 2026-07-03) brings stackful coroutines: `create` /
`resume` / `yield` / `status` / `wrap` / `running` + error
propagation, sized 11-15 sessions across N5-A..E. N5-A lays the
runtime foundation and the `coroutine.create` value.

## Decisions

### 1. Context-switch core in C, not the Rust bridge

The bridge object (ADR 0191) is `#![no_std]` Rust and can declare
extern libc symbols — but `makecontext`/`swapcontext` require
filling `ucontext_t` fields (`uc_stack`, `uc_link`) whose layout is
libc-private and arch-specific. Hand-declaring that layout in Rust
is exactly the fragility the ADR 0126 posture avoids. So the
coroutine core is a small C file (`src/runtime/coroutine.c`) using
`<ucontext.h>`, compiled by `build.rs` with the host `cc -c`
(already a hard runtime dependency of the compiler) and linked into
every generated binary next to `bridge_runtime.o` (same
`LUMELIR_BRIDGE_OBJ` mechanism, second object).

### 2. ucontext first, linux-gated; darwin is a bake question

glibc ships working `ucontext` on both x86_64 and aarch64 (both
linux CI lanes). Apple deprecated the API and its arm64-darwin
behaviour is unverified — and the darwin lane is a hard gate
(ADR 0311). Coroutine e2e tests are therefore `cfg`-gated off on
macOS until a darwin bake proves `ucontext` there; if it fails, the
recorded fallback is a minicoro-style naked-asm switch (one ~40-line
routine per arch), a later ADR.

### 3. Coroutine object + tag

`lumelir_coro_create(fn_ptr, cell_ptr)` mallocs
`{ ucontext_t self; ucontext_t caller; void *stack; int64 status;
fn_ptr; cell_ptr; xfer slots }`, allocates a fixed 256 KiB stack
(revisit under N8), and `makecontext`s a trampoline that will invoke
the Lua closure through the cell-ptr-first ABI (ADR 0083) on first
resume (N5-B). Status lifecycle: `0 suspended → 1 running → 2 dead`
(+ `3 normal` when it has resumed another coroutine, N5-D).

Tagged representation: **`TAG_COROUTINE = 8`** (next free after
`TAG_USERDATA = 7`, ADR 0261), payload = object pointer. `type(co)`
reports `"thread"`. The object itself is plain `malloc` for N5-A —
GC integration (a `GC_TYPE_COROUTINE` header + stack scanning) is
deferred to the N5-E/N8 boundary and recorded as a known leak until
then. Codegen template: the `Newproxy` arm (ADR 0261) — evaluate the
Function arg to its closure cell ptr, read the fn ptr via
`emit_load_closure_fn_ptr`, call `lumelir_coro_create(fn_ptr, cell)`,
pack the result with the new tag.

### 4. HIR surface

`Builtin::CoroutineCreate` (namespace `coroutine`, method `create`,
arity 1). The argument must be a Function value; codegen resolves it
to its closure cell ptr (singleton global for non-capturing fns) and
passes `(fn_ptr, cell_ptr)` to the runtime. `ret_kinds =
[TaggedValue]` carrying the new tag. `resume`/`yield`/`status`/
`wrap`/`running` are N5-B..D and get their own ADRs as the ABI
questions surface.

## Bake log (2026-07-14)

Landed as designed (C core, tag 8, Newproxy-template arm, `type()` /
print-typename / `==`-identity chains extended). Two pre-existing
boundaries surfaced by the new tag, both shared with `TAG_USERDATA`:

1. **Real bug, fixed**: the IndexAssign tagged-value placeholder
   (`emit.rs`, ADR 0138-M) applied only to TaggedValue *keys*, so a
   String-key store of a tagged local went through `emit_expr`'s
   Number-payload guard — trapping ANY non-Number tag (tagged
   String / userdata / coroutine) at store time. Condition widened
   to every TaggedValue value; `newproxy()` values become
   hash-storable for the first time too.
2. **Inherited scope line, kept**: the array path (`t[1] = co`)
   still rejects TaggedValue values (array slots need a concrete
   kind for grow-extend, ADR 0138-M). Hash keys work.

## N5-A acceptance

- `local co = coroutine.create(function() end); print(type(co))` →
  `thread` (linux lanes).
- New tag round-trips through locals, table slots, and `==`
  identity.
