# CONTRACT.md -- halmi-script

## Purpose and layer

The JavaScript/TypeScript adapter for halmi: one lexer, one structure pass,
one classifier. A `Dialect` (`Js`, `Jsx`, `Ts`, `Tsx`) selected from the
file extension controls the keyword table, JSX enablement, and the reported
language; everything else is shared code. Implements the shared
`halmi_core::adapter::LanguageAdapter` port: pure, never panics, degraded
files become error entries.

## Public API

- `ScriptAdapter` -- `language() == "JavaScript/TypeScript"`, `analyze(path, src)`.
- `Dialect` -- `from_path`, `has_jsx`, `is_ts`, `language`.

## Classification (token-faithful)

- Operators: punctuators except separators and brackets (`; , : ( ) [ ]`),
  plus `call`, `index`, `attr`, plus keywords (control, declaration, and TS
  type keywords). Every analyzed function carries exactly one `function`
  declaration operator (methods and arrows included, for cross-language
  comparability with the Rust adapter's `fn`); arrows additionally count
  their `=>` operator.
- Operands: identifiers, numbers, strings, template chunks, regex literals,
  JSX text.
- Cyclomatic complexity: 1 + `if`, `for`, `while`, `do`, `case`, `catch`,
  `&&`, `||`, `??`, ternary `?`.
- Logical LOC: non-blank, non-comment lines in the span.
- Nesting: brace depth inside the body.
- `io_params`: distinct parameter names plus one for a TS return annotation.
- `method_chain`: consecutive called selectors on one spine.
- Comment density: `//` and `/* */` lines in the span (always `Some`).

## Lexical context (the only thing JS needs beyond plain lexing)

- `/` is a regex when the previous significant token is in expression
  position; a small brace-kind stack (block/object/class) resolves the `}`
  case (object literal `}` divides, block `}` starts a regex).
- Template literals: `${` opens an expression context, the matching `}`
  returns to the template; nested templates stack.
- JSX (`.jsx`/`.tsx` only): `<` in expression position enters markup; tag
  depth is tracked; `{...}` inside markup is an expression context; text is
  operands. Unclosed markup refuses the file.
- TS type syntax is lexed as ordinary tokens; `<T>` is `<`, `T`, `>`.

## Invariants

1. Never panics: unterminated strings/templates/regex/block comments/JSX
   degrade the whole file to `error` with `functions: []`.
2. Same pipeline, same contract: `reports_from_raw` -> `finalize` works
   unchanged; deterministic and idempotent.
3. Nested functions (callbacks, arrows) are reported and counted inside
   their parent, matching the Go adapter's closure rule.

## Error model / determinism / cancellation / unsafe / features

Single optional `error` string per file (lex errors only; no IO in this
adapter). Deterministic single pass, report order = source order.
Synchronous, bounded by input. `#![deny(unsafe_code)]`. No feature flags.

## Conformance tests

`tests/script.rs` -- JS spans/names/counts, regex vs division, TS types and
return annotations, TSX JSX, shared-pipeline determinism, degradation,
unicode identifiers.

## Oracle differential (test-only, outside the repo)

`tools/oracle_diff.py` compares function spans against a private tree-sitter
dump tool that is not included in this repository (test-only, not a product
dependency). The figures in this section are historical local runs,
UNREPRODUCIBLE from this repo (missing harness: the oracle binary; the
corpus is not vendored). Most recent observed run: with `script` mode,
287/287 exact spans in its first 40 files. A larger invocation on the same
local pi-stack corpus produced 7,529/7,529 exact spans across 349 files.
Missing source files or an oracle failure is an error, not a passing
comparison.

## Negative ledger (known gaps, retry conditions)

1. **Two TSX files in a 197-file real corpus degrade** (1%) with
   "unterminated JSX" / "newline in regex": a JSX depth desync on a
   construct not yet isolated (the files are valid TSX; the lexer refuses
   them rather than emitting wrong numbers). Retry when the construct is
   bisected; the oracle differential on the 200-file pi-stack sample is
   still 4,498/4,498 exact.

Historical nearby result, 2026-11-21: the oracle differential reported
4,498/4,498 exact spans on a 200-file sample. That is a different fixture and
must not be substituted for the current `oracle_diff.py` output above, nor
used to imply that every older TSX gap is permanently resolved without
identifying an actual regression fixture.

Closed in this pass:
- Return-type object literals (`function f(): { a: string } | null {`)
  resolve the body brace via a type-position discriminator.
- Type aliases, interfaces, and `declare` statements are masked for arrow
  detection (statement-position check, ASI-aware; `>>`/`>>>` close type
  depth).
- `:`-introduced type annotations are masked using the lexer's brace kinds
  (object-literal `:` and ternary `:` are excluded); `as`/`satisfies`
  expressions and `extends` constraints are masked.
- Methods with computed names (`[Symbol.asyncIterator]() {}`) and methods
  literally named `get`/`set`/`static` are detected.
- Arrow expression bodies that are ternaries are spanned correctly
  (ternary depth tracked; the outer ternary's `:` ends the body).

## No-claim boundaries

- JSX text/attributes are approximated (text = operands, attributes =
  operators/operands).
- ASI means LOC is line-based, not statement-based.
- Decorators and TS type-level constructs are lexed as ordinary tokens.
