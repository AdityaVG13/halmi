# halmi-swift CONTRACT

## Purpose and layer

Language adapter for Swift. Hand-written lexer (no tree-sitter, no syn,
no parser dependency): token-faithful single pass over the source
producing one `RawFunction` per function-like declaration, fed through
the shared `halmi-lex::MetricsCollector` for Halstead/CC/LOC/nesting/
chain signals.

## Counted declarations

`func` (free, method, operator, backticked, keyword-named like
`func open()`), `init`, `deinit` -- declarations only: call-site
`super.init(...)`, `.init($0)`, and `Foo.init(...)` are skipped via a
member-boundary check. Methods are qualified `Type.name` (innermost
enclosing class/struct/enum/protocol/extension/actor). Excluded:
closures, computed-property accessors (`get`/`set`/`willSet`/`didSet`),
`subscript`, `precedencegroup`, and bodyless protocol requirements
(`protocol_function_declaration` in tree-sitter terms; matches the Rust
and Go adapters, which do not count trait/interface members without
bodies). Nested functions are counted. Spans match tree-sitter-swift's
`function_declaration` / `init_declaration` / `deinit_declaration`
nodes (differential: 5,350/5,355 exact across 575 files from five local
corpora; the residual is tree-sitter grammar failures on nested raw
strings inside interpolations and `#if arch()` cascades, where halmi is
correct and the oracle is not).

Spans start at the first modifier or attribute of the declaration
(matching tree-sitter, which folds `modifiers` and `attribute` into the
node) and end at the closing brace, or at the last signature token for
bodyless protocol members.

## Classification

* Operators: Swift symbol operators via the shared C-family punctuator
  table (`+ - * / % = == != < > <= >= && || ! & | ^ ~ << >> ?? ?. ..<
  ... -> += -= ...`), implicit `call` and `index` operators, and the
  keywords in `KEYWORD_OPERATORS` (control flow plus
  declaration-shaping keywords). Modifiers (`public`, `static`, ...)
  are recognized keywords but not operators.
* Operands: identifiers (including `self`, `nil`, `true`, `false`,
  `$0`, keypaths `\.member`), numbers, string literals, and regex
  literals. Strings collapse `\(expr)` / `\#(expr)` interpolation
  (recursively, including nested raw strings) into one operand;
  regular `"..."`, multiline `"""..."""`, and raw `#"..."#`..`##"..."##`
  forms are scanned. Regex literals `#/.../#` (any number of `#`s) are
  one operand; a bare `/.../` is lexed as a regex only where an
  expression can start (after an operator or keyword), otherwise as
  division.
* Cyclomatic complexity: 1 + one per `if`, `guard`, `for`, `while`,
  `case`, `where`, `&&`, `||`. `repeat` is covered by its trailing
  `while`; `default` adds nothing.
* Physical LOC: distinct lines carrying body tokens (declaration header
  line and final closing brace excluded), identical to the Go adapter.
* Nesting: brace depth inside the body (body = 0).
* `io_params`: top-level parameter count plus 1 when a `->` result
  exists.
* `method_chain`: consecutive method calls where each receiver is the
  result of the previous call. Newlines reset the chain (multi-line
  chains undercount by segment), and force-unwraps (`x!.foo()`) do not
  extend chains.

## Invariants

* Pure function of `(path, src)`; no IO, no panics on any input (fuzz
  property: lex errors degrade to `error`, never a crash).
* Deterministic: same input, same output, independent of environment.
* Nested functions are reported (each `func` token counts), with spans
  inside the parent span.

## Error model

Any unterminated construct (string, raw string, multiline string,
interpolation, block comment, backtick identifier) or unexpected
character produces a single `error` string with the line number; the
file yields zero functions. Files are independent: one bad file never
affects another.

## Determinism class

Fully deterministic single pass; no ordering effects, no hashing of
iteration order in outputs (operator/operand maps are aggregated into
sorted counts before reporting).

## Cancellation

Not applicable: synchronous single-file analysis with no cancellation
points. The CLI bounds parallelism at the thread-pool layer.

## Unsafe boundary

None. `#![deny(unsafe_code)]`.

## Feature flags

None.

## Conformance

`src/lib.rs` has 1 keyword-table test and `tests/swift.rs` has 7 additional
focused integration tests, 8 total: adapter-port spans, names, counts and
ranking; string interpolation and multiline strings; backticks and operator
names; modifier-line spans; and degradation. Span parity with tree-sitter-swift
was measured by `tools/oracle_diff.py` in swift mode over local corpora with a
private oracle binary that is not in this repository; those figures are
historical and UNREPRODUCIBLE from this repo.

## No-claim boundaries

* Generic `where` clauses on type declarations inflate CC by their
  `where` (a decision in for-loops, a constraint on types).
* Hexadecimal floats (`0x1.8p3`) lex as a number plus stray tokens.
* `#if`/`#endif` compiler conditionals are not branches (tokens).
* Bodyless signatures split across lines (rare protocol style) can end
  early at the first depth-0 newline.
* Attribute contents inside `@( )` are skipped as one unit; unusual
  attribute shapes (`@objc(foo)`, `@available(*, deprecated, message:
  ...)`) are handled only when balanced parentheses close them.
