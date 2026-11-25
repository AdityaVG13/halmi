# halmi-zig contract

## Purpose and layer

Zig language adapter for halmi: hand lexer + declaration scanner built
on the shared `halmi-lex` machinery (`MetricsCollector`, `cfamily`
punctuators). Zero dependencies; no tree-sitter anywhere in the
product (the tree-sitter-zig binary is a test-side differential
oracle only).

## Counted declarations

* `fn` declarations, including `pub`/`export`/`extern`/`inline`/
  `noinline`/`threadlocal`/`callconv(...)`/`linksection(...)`/
  `align(...)` modifier runs (folded into the span, matching
  tree-sitter-zig) and bodyless `extern fn ...;` forms.
* `test` blocks: `test "name" {}`, unnamed `test {}`, and identifier
  decltests (`test portable {}`).
* A `fn` in type position (`*const fn (u32) u32`) is not counted; the
  discriminator is a missing identifier between `fn` and `(`.
* Anonymous `struct {...}` return types are part of the signature, not
  the body.
* Qualification: `Container::name` where the container is the
  innermost `const NAME = struct|enum|union|error|opaque { ... }`.

## Metric rules

* Halstead: identifiers (including `@builtin`s and `@"raw idents"`),
  numbers, strings, and chars are operands; control keywords
  (`if`, `else`, `while`, `for`, `switch`, `return`, `break`,
  `continue`, `defer`, `errdefer`, `try`, `catch`, `orelse`, `and`,
  `or`, `suspend`, `resume`, `await`, `unreachable`) are operators.
* Cyclomatic complexity: 1 + one per `if`, `while`, `for`, `catch`,
  `and`, `or`.
* Physical LOC; nesting = brace depth inside the body (body = 0).
* `io_params`: distinct parameter names + 1 when the return type is
  not exactly `void`.
* Method chains follow the shared `MetricsCollector::call(is_method,
  extends)` semantics.

## Lexing

Line comments `//` only (Zig has no block comments); regular strings
with `\` escapes; `\\`-prefixed multiline strings (each line one
token); char literals; `@"raw identifier"`; numbers with `0x`/`0o`/
`0b` prefixes, `_` separators, floats and exponents. Newlines are
tokens (Physical LOC via code-line tracking).

## Error model

Unterminated strings/chars, stray characters, and functions without a
body terminator surface as structured per-file errors; other functions
in the file are still reported.

## Determinism class

Single pass over the token stream; no shared state; order of reported
functions follows source order.

## Conformance

Differential against tree-sitter-zig (oracle): 22,112/22,112 exact
spans across 1,982 files (100%). Historical, maintainer-local run with a
private oracle binary and an unvendored corpus: UNREPRODUCIBLE from this
repo. Unit tests in `tests/zig.rs`
(declaration kinds, modifier spans, fn types, nesting, chains, lexer
errors, raw identifiers), mutation-verified on the modifier-run span.

## No-claim boundaries

* Generic/callconv syntax beyond the listed modifier forms is
  validated only by the corpus differential.
* `asm` blocks are tokenized as plain tokens.
* No semantic type checking; Zig build errors may shift spans in
  malformed files.
