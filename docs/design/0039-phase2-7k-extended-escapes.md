# 0039. Phase 2.7k: Extended String Escapes

- **Status:** Accepted
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Phase 2.7a (ADR 0024) shipped a minimal escape set:
`\n \t \r \\ \" \' \0`. That covers the common formatting cases
but leaves three gaps that Lua source in the wild routinely
relies on:

- **C-style control escapes** `\a \b \f \v` — bell, backspace,
  form-feed, vertical-tab. Cheap to add.
- **`\xHH` hex byte** — common for embedding control bytes or
  matching binary protocols by literal value.
- **`\ddd` decimal byte** — 1-3 decimal digits. Useful for
  porting code that uses the decimal form.

Lua also defines `\u{XXXX}` (UTF-8 codepoint) and `\z`
(whitespace skip). Both are deferred — see *Out of Scope*.

## Decision

### Two new pure helpers

The escape `match` arm in `scan_string` gains:

```rust
'a' => '\x07',   'b' => '\x08',
'f' => '\x0C',   'v' => '\x0B',
'x' => read_hex_escape(chars, esc_off)?,
d if d.is_ascii_digit() => read_decimal_escape(chars, d, esc_off)?,
```

`read_hex_escape` consumes exactly two hex digits and returns
the byte. `read_decimal_escape` takes the already-consumed first
digit plus up to two more and returns the byte. Both are pure
relative to their inputs (only the chars iterator advances).

The pre-existing `'0' => '\0'` arm is retired — it becomes a
strict subset of the decimal handler (`\0` with no following
digit yields value 0). This deletion preserves all prior `\0`
semantics.

### ASCII-safe range restriction (0..=0x7F)

Both numeric escapes reject values above 127 with
`LexError::InvalidEscape`. The reason: Lua strings are *byte*
strings, but our HIR/codegen carry `String` (UTF-8). A value of
0xC3 pushed as `char` into a Rust `String` UTF-8-encodes to two
bytes (`0xC3 0x83`), which is **not** what Lua means. Restricting
to ASCII keeps every escape one byte without opening the
byte-vs-UTF-8 question prematurely. Full byte-string support
is deferred to whatever phase introduces a `Vec<u8>`-backed
string runtime.

### `\ddd` is greedy but stops at non-digits

`read_decimal_escape` consumes up to two more digits after the
first, and stops at the first non-digit. So `\65X` produces
`AX` (the `X` survives into the string body), and `\065` is
allowed for explicit zero-padding. Decimal scanning never
crosses a `\` — only digit characters in the source after the
backslash count.

### CA invariants preserved

| Layer    | Change                                                  |
|----------|---------------------------------------------------------|
| Lexer    | Two pure helpers (`read_hex_escape`, `read_decimal_escape`); five new escape arms; `'0'` arm retired |
| Parser   | None                                                    |
| AST      | None                                                    |
| HIR      | None                                                    |
| Codegen  | None                                                    |

The escaped-byte value flows through the existing
`TokenKind::Str(String)` channel — no token, AST, HIR, or codegen
shape changes.

## TDD Process

1. **Red**: 9 lexer unit tests + 8 integration tests added,
   covering each new escape, the boundary cases (`\xff`, `\200`,
   `\xZ`), and the greedy-but-bounded `\65X` case. Six unit tests
   failed because the impl rejected the new escape characters as
   `InvalidEscape`; three already passed because they expected
   `InvalidEscape` (boundary protection).
2. **Green**: added `\a \b \f \v` mappings, then the two helpers,
   then retired the `'0' => '\0'` arm (a stale special case
   shadowing the new digit handler — caught by the
   `lex_string_with_decimal_escape_three_digits` test). All
   tests passed at 545.
3. **Refactor**: doc comment on `scan_string` updated to enumerate
   the new escape set. No further duplication emerged — the
   helpers are each called from exactly one site.

## Alternatives Considered

- **Allow 0..=0xFF and rely on UTF-8 multi-byte encoding**.
  Diverges from Lua's byte-string semantics; would silently
  produce two-byte sequences for escapes like `\xff`. Rejected —
  the boundary error surfaces the limitation honestly.
- **Switch the lexer's accumulator to `Vec<u8>` immediately**.
  Required for full Lua semantics but a non-trivial refactor
  through HIR (which currently treats strings as `String`) and
  codegen (which uses C string runtime). Defer until raw-byte
  use cases actually arrive.
- **Implement `\u{XXXX}` here**. The codepoint→UTF-8 conversion
  is straightforward but the test surface (multi-byte `#s`
  results, equality across normalised forms) is its own thing.
  Defer.
- **Extract a unified `read_escape_char` matching all cases**.
  The two helpers are already cohesive on a single concern (hex
  vs decimal); merging them would create a switch on a shape
  parameter. Rule of three not yet met.

## Consequences

- Lexer adds ~60 lines (two helpers + four arms).
- 9 lexer unit tests + 8 integration tests; total green at 545.
- ADR 0024's "escape set" enumeration is now obsolete —
  authoritative list is in `scan_string`'s doc comment.

## Out of Scope

- **`\u{XXXX}`** — UTF-8 codepoint. Adds multi-byte length
  semantics; deferred until a use case demands it.
- **`\z`** — whitespace-skip escape, used in long string-like
  patterns. Useful but niche.
- **Numeric escapes >127** — pending raw-byte string runtime.
- **`\<newline>`** — literal newline continuation. Niche; defer.
