# 0153. `pcall` / `error` Value Propagation — Phase 3 Trigger

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-06-01
- **Deciders:** ShortArrow

## Context

[ADR 0133](0133-phase2-completion-criteria.md)'s deferral table lists "`pcall` / `error` value propagation" as a remaining workstream. The metatables base is now complete (ADRs 0134 – 0151), so the "error-table shape depends on metatables" precondition is satisfied.

Today `error(msg)` traps with `s_user_error` (ADR 0033). `pcall` is not lowered — `pcall(f)` rejects at HIR with `UnknownFunction`. The two pieces together — protected calls + value-carrying error throw — are an architecturally significant unit of work.

Per ADR 0133's contract ("Phase 2 closes when each workstream has an ADR; implementation may stagger into 2.7+"), this ADR records the **strategy decision** and pins implementation to Phase 3.

## Decision

**Phase 2 freezes the current behaviour. Phase 3 adds protected calls via setjmp/longjmp at the C-ABI boundary.**

Concretely:

1. **Phase 2 = current behaviour preserved.**
   - `error(msg)` continues to trap with `s_user_error` (ADR 0033). The trap is unconditional; `pcall` cannot recover from it.
   - `pcall(...)` and `xpcall(...)` continue to reject at HIR (`UnknownFunction`). Users that need them today must restructure their code to avoid throwing.

2. **Phase 3 = setjmp/longjmp ABI.**
   - Module declares `setjmp` + `longjmp` libc externs.
   - Thread-local (initially process-global) `jmp_buf` + error-value slot.
   - `error(msg)`: stash `msg` in the error-value slot, call `longjmp(jmpbuf, 1)`.
   - `pcall(f, args...)`: save current jmpbuf state; `setjmp(jmpbuf)`; if 0 returned, dispatch `f(args...)` and return `(true, f-results...)`; if 1 returned, return `(false, error-slot)`.
   - Multi-return ABI from `pcall` follows the existing ADR 0021 / 0076 multi-result-position pattern.
   - Error value: String only at Phase 3 first cut; Table-form (`error({code=42, msg="..."})`) deferred per Lua spec error-table shape (depends on `__tostring` chain — RESOLVED by ADR 0142, so the Phase 3 ADR may bundle).

3. **No reference counting / panic / unwinding alternative.** setjmp/longjmp matches the Lua reference implementation; consistent with our existing libc-extern call patterns.

### What Phase 3 ADR owns

When the Phase 3 ADR lands, it will:

- Add `Builtin::Pcall` (and possibly `Builtin::Xpcall`) HIR variants.
- Register `setjmp` / `longjmp` libc externs in `emit_module_init`.
- Add a thread-local-style `g_jmpbuf` + `g_error_msg` global (or grow the existing `s_user_error` into a runtime-mutable buffer).
- Rewrite `error(msg)` codegen: stash into `g_error_msg`, call `longjmp(g_jmpbuf, 1)`.
- Add `Callee::Builtin(Pcall)` emit arm: nested `setjmp` save/restore + scf.if(returned == 0).
- Multi-return ABI for `pcall` reusing ADR 0021 / 0076 patterns.

### Trigger to implement

Any of:

- (a) An external user requests `pcall` for a real program.
- (b) `string` patterns (ADR 0155) lands and benefits from `pcall`-protected dispatch.
- (c) `_ENV` / true globals (ADR 0154) lands and exposes a runtime error path that needs protection.

## Alternatives considered

- **`pcall` as a no-op wrapper that returns `(true, f(...))` and lets `error()` continue to trap.** Rejected — silent dishonesty; user code that tests `pcall` expects actual protection.
- **HIR-only static analysis: pre-walk f's body for `error()` calls and reject `pcall(f)` when found.** Rejected — undecidable for indirect calls / loops / metamethod recursion.
- **`panic`-style abort with `s_user_error`-as-message.** Rejected — doesn't carry values back to the caller (Lua spec violates).
- **Implement now in Phase 2.** Rejected — setjmp/longjmp is non-trivial (jmpbuf size is libc-specific, requires a dedicated runtime slot, and the multi-return ABI integration is its own ADR). Honors the ADR 0133 contract.

## Consequences

**Positive**
- ADR 0133's `pcall` / `error` row is satisfied; Phase 2 close criterion advances.
- Future ADR has a clear plan to cite.
- No new ABI shift in Phase 2.

**Negative**
- User-visible feature gap stays in Phase 2. Documented in this ADR.
- Phase 3 implementation will be M/L sized.

**Locked in until superseded**
- setjmp/longjmp as the Phase 3 mechanism (no reference counting, no panic-style unwinding).
- Thread-local-style jmpbuf (process-global initially).
- String-form error value first cut; Table-form deferred.

## Documentation updates

- [x] §4 LIC — new `LIC-pcall-error-strategy-1`.
- [x] §7 — closes `pcall` / `error` open item with Phase 3 trigger.
- [x] §8 — adds 0153.
- [x] ADR 0133 deferral table — `pcall` row gets `RESOLVED by ADR 0153 (Phase 3 trigger)` annotation.

## Test count delta

```
Step 0:   1366 (after ADR 0152)
C1 (this ADR + SoT updates): 1366 → 1366 (docs only)
```

Decision-only ADR. No code changes.

## Critical files

- `docs/design/0153-pcall-error-strategy.md` (this file).
- `docs/design/tagged-semantics.md` — §4 / §7 / §8.
- `docs/design/0133-phase2-completion-criteria.md` — deferral row annotation.
- `docs/design/README.md` — index entry.

## Risks

| Risk | Mitigation |
|---|---|
| Phase 3 setjmp/longjmp ABI breaks the existing closure-cell ptr passing | Phase 3 ADR will audit ADR 0083 cell-ptr threading and the multi-return ABI before landing. |
| Thread-local jmpbuf interacts with coroutines (Phase 3+) | Coroutines are a separate ADR; both will share the same TLS slot mechanism. |
| User code that relies on the current `error()` trap behaviour breaks when Phase 3 lands | Phase 2 binary continues to use the trap path. Phase 3 is opt-in via the implementation ADR; not enabled retroactively in Phase 2 binaries. |

## Future work

- Phase 3 implementation ADR — see "What Phase 3 ADR owns" above.
- `xpcall(f, handler, ...)` — bundled with the implementation ADR.
- Table-form error values (`error({...})`) — depends on `__tostring` chain RESOLVED by ADR 0142; may bundle into the same ADR.
- Coroutines (`coroutine.*`) — shares the TLS slot mechanism.

## References

- [ADR 0033](0033-phase2-7h-error.md) — current `error(msg)` trap.
- [ADR 0021](0021-phase2-5d-multi-return.md) — multi-return ABI (used by `pcall`).
- [ADR 0076](0076-phase2-6c-tag-locals-fn-multi.md) — multi-position TaggedValue ABI.
- [ADR 0083](0083-phase2-5c-full-closures.md) — closure cell ptr threading.
- [ADR 0133](0133-phase2-completion-criteria.md) — deferral table row this ADR closes.
- [ADR 0142](0142-tostring-metamethod.md) — `__tostring` chain that Table-form errors depend on.
- Lua 5.4 reference manual §6.1 — `pcall` / `xpcall` / `error` semantics.
