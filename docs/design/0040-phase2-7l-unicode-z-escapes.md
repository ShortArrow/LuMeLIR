# 0040. Phase 2.7l: `\u{XXXX}` Codepoint and `\z` Whitespace-Skip

- **Status:** Accepted
- **Date:** 2026-05-02
- **Deciders:** ShortArrow

## Context

Phase 2.7k (ADR 0039) closed the byte-oriented escape gaps:
`\a \b \f \v \xHH \ddd`. Two more escape forms remain:

- **`\u{XXXX}`** — the only Lua escape that produces *multiple
  bytes* per occurrence (UTF-8 encoding of a Unicode codepoint).
  Common in source files containing real text in any non-ASCII
  language.
- **`\z`** — skip a run of whitespace (spaces, tabs, newlines)
  starting immediately after the escape. Lets long string
  literals split across lines without baking line breaks into
  the value.

Both are pure lexer changes and slot into the `match esc_ch` /
pre-match shape established in 2.7k.

## Decision

### `\u{XXXX}` via codepoint → `char` → UTF-8

Rust's `char` *is* a Unicode scalar value, and
`String::push(char)` does the UTF-8 encoding. So the helper
returns a `char`, the existing `value.push(mapped)` call stays
unchanged, and multi-byte expansion happens for free:

```rust
fn read_unicode_escape<I>(
    chars: &mut Peekable<I>,
    esc_off: usize,
) -> Result<char, LexError>
where I: Iterator<Item = (usize, char)>;
```

The helper:

1. Requires the `{` immediately after `u`.
2. Reads 1+ ASCII hex digits into a `u32` (`checked_mul`/
   `checked_add` so a malicious overflow case errors cleanly).
3. Requires `}`.
4. Validates via `char::from_u32` — that one check excludes
   surrogates `0xD800..=0xDFFF` and values >`0x10FFFF` in a
   single call.

Each failure point produces a distinct
`LexError::InvalidEscape { seq, offset }` with `seq`
distinguishing "missing `{`", "missing `}`/non-hex inside",
"empty digit run", and "out-of-range scalar".

### `\z` as a pre-match short-circuit

`\z` doesn't push a value — it consumes whitespace from the
char stream and rejoins the surrounding loop:

```rust
if esc_ch == 'z' {
    while let Some(&(_, c)) = chars.peek() {
        if c.is_ascii_whitespace() {
            chars.next();
        } else {
            break;
        }
    }
    continue;
}
```

Placed *before* the `match esc_ch` so the post-match
`value.push(mapped)` doesn't run. `is_ascii_whitespace` covers
space, tab, CR, LF, VT, FF — the same set Lua treats as
whitespace inside `\z`.

### Legacy test rotation

The Phase 2.7a test `lex_invalid_escape_returns_error` used
`\z` as its sample unrecognised escape. With `\z` now valid,
the test was rotated to `\q`. No coverage lost — the test
still pins the unrecognised-escape diagnostic shape.

### CA invariants preserved

| Layer    | Change                                                  |
|----------|---------------------------------------------------------|
| Lexer    | One pure helper (`read_unicode_escape`); pre-match `\z` short-circuit; one `match` arm for `\u`. Existing helper `read_decimal_escape` and friends untouched. |
| Parser   | None                                                    |
| AST      | None                                                    |
| HIR      | None                                                    |
| Codegen  | None                                                    |

`\u{XXXX}` may now produce strings whose char count differs
from their byte count. Lua's `#s` is byte-length and our
runtime already calls `strlen`, so existing semantics line up
with Lua's "byte string" view. No HIR or codegen change is
needed because nothing in the pipeline assumes "1 char = 1
byte".

## TDD Process

1. **Red**: 9 lexer unit tests + 8 integration tests added —
   ASCII / 2-byte / 3-byte / 4-byte codepoints, `\z` skipping
   newline + spaces, `\z` no-op at EOS, and the four
   error-shape boundaries (missing `{`, missing `}`, empty
   digits, surrogate). 6 new behaviour tests failed; 3 boundary
   tests passed because the existing impl rejected unknown
   escapes.
2. **Green**: added the pre-match `\z` branch and the
   `read_unicode_escape` helper. Rotated the legacy test from
   `\z` → `\q`. All tests passed at 562 (554 + 8).
3. **Refactor**: doc comment on `scan_string` updated to
   enumerate the full set across ADRs 0024 / 0039 / 0040.

## Alternatives Considered

- **Push UTF-8 bytes directly via `Vec<u8>`** instead of
  `char::from_u32` + `String::push`. Required only if HIR
  changes its string carrier, which it hasn't. Rejected.
- **Allow surrogate pairs `\u{D83D}\u{DE00}` to combine into a
  non-BMP codepoint**. Lua doesn't do this; rejecting
  surrogates outright matches the spec.
- **Restrict `\z` to long-bracket strings**. Some references
  document `\z` as long-string-only, but Lua 5.4 allows it in
  short strings, which is where it's actually useful (long
  strings are already raw multi-line). Followed Lua 5.4.
- **Treat `\u{...}` as a single token of `Vec<u8>`** to skip
  the codepoint→char step. The current path is one
  `from_u32` + one `String::push`; nothing to optimise.

## Consequences

- Lexer adds ~50 lines (one helper + a `\z` branch + the `\u`
  match arm).
- 9 lexer unit tests + 8 integration tests; total green at 562.
- The escape-set table in `scan_string`'s doc comment is now
  the authoritative source — older ADRs (0024, 0039) are no
  longer canonical for the full set.

## Out of Scope

- **Codepoints >`0x10FFFF`** — Lua *spec* permits up to
  `0x7FFFFFFF` (the historical UTF-8 range), but our `char`
  pivot bounds us at `0x10FFFF` (Unicode-valid). A future
  raw-byte-string runtime would lift this.
- **Mid-string codepoint validation hooks** (e.g. emitting NFC).
  Not a Lua concern.
- **`\<newline>`** — literal newline continuation escape. Niche
  alternative to `\z`; defer.
