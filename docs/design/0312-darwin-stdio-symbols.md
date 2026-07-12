# 0312. Platform-dependent libc stdio global symbols

- **Status:** Accepted
- **Kind:** Architecture Decision
- **Date:** 2026-07-12
- **Deciders:** ShortArrow

## Context

The darwin lane's first full test run (ADR 0311 bake, round 6) failed
every e2e test at the generated binary's link step:
`Undefined symbols: "_stdout"`. Generated code declares the libc
`FILE *` globals by their glibc export names (`stdout`, `stdin`;
ADR 0117/0119) — but Apple's libSystem exports them as `__stdoutp` /
`__stdinp`, with the POSIX names existing only as C macros. ADR 0117
explicitly scoped macOS out; N9-C brings it in scope.

## Decision

`stdio_symbol(posix_name, macos)` in `codegen::emit` is the single
mapping point: `stdout`→`__stdoutp`, `stdin`→`__stdinp` on darwin,
identity elsewhere. `host_stdio_symbol` selects via
`cfg!(target_os = "macos")` — generated code links against the host
libc because `llc` targets the host triple (ADR 0308), so the
compiler's own target OS is the right discriminator. All four emit
sites (extern-global decls + addressof reads) go through it.

The pure two-arg function keeps both arms unit-testable from any
platform; the darwin arm's end-to-end proof is the arm64-darwin CI
lane itself. Windows (`__acrt_iob_func()`) stays out of scope until a
Windows lane exists.

## Consequences

- First codegen (not infra) divergence keyed on target platform. If
  more accumulate, promote the pattern to a target-descriptor struct
  rather than scattering `cfg!` calls.
