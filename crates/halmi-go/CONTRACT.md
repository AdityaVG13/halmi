# CONTRACT.md -- halmi-go

## Purpose and layer

The Go adapter for halmi: a hand-written lexer (no parser dependency)
feeding one structure pass that yields `RawFunction`s. Implements the
shared `halmi_core::adapter::LanguageAdapter` port: pure, never panics,
one pass, degraded files become error entries. Everything downstream is
shared.

## Public API

- `GoAdapter` -- `language() == "Go"`, `analyze(path, src)`.

## Classification (token-faithful)

- Operators: Go symbol operators (`+ - * / % & | ^ << >> &^ && || ! == !=
  < <= > >= = := += ... <- ++ -- ...`), `.`, `call`, `index`, and the
  keywords `func if else for range switch case select return break
  continue go defer chan map struct interface`. Every analyzed function
  carries exactly one `func` declaration operator (mirrors the Rust
  adapter's `fn`); function literals are part of their parent. Separators and brackets
  (`, ; : ( ) [ ] { }`) are excluded.
- Operands: identifiers that are not keywords, numbers, interpreted and
  backtick-raw strings, and runes.
- Cyclomatic complexity: 1 + one per `if`, `for`, `case`, `select`,
  `&&`, `||`.
- Logical LOC: non-blank, non-comment lines in the span (closing brace
  excluded).
- Nesting: brace depth inside the body (body = 0).
- Function literals (closures) count inside their parent, like Rust.
- Body-less declarations (`func name(signature)` with no block, the
  assembly-implemented form the Go spec allows) are signatures, not
  definitions: not counted, like Rust trait method declarations and C
  prototypes. The declaration ends at its signature line (a newline after
  the parameter list is the terminator, mirroring Go's semicolon
  insertion), so the declarations after it are scanned normally.
- Spans: the `func` line through the closing brace line.
- `io_params`: distinct parameter names (receiver excluded) plus one when
  the result list is non-empty. `method_chain`: consecutive called
  selectors on one spine.
- Comment density: `//` and `/* */` lines in the span (always `Some`).

## Invariants

1. Never panics: unterminated strings/raw strings/runes/block comments
   degrade the whole file to `error` with `functions: []`. Line counters
   are guarded against drift (raw strings advance lines exactly once).
2. Same pipeline, same contract: `reports_from_raw` -> `finalize` works
   unchanged; deterministic and idempotent.
3. Methods are qualified `Type.method` (receiver's last type identifier).

## Error model / determinism / cancellation / unsafe / features

Single optional `error` string per file (lex errors only; no IO in this
adapter). Deterministic single pass, report order = source order.
Synchronous, bounded by input. `#![deny(unsafe_code)]`. No feature flags.

## Conformance tests

`tests/go.rs` (12 tests) -- spans/names/counts, receiver methods, raw
strings and runes, shared-pipeline determinism, io_params from signatures,
line-drift regression, degradation, body-less declarations (failure-first:
before the fix a stub swallowed every declaration up to the next body).
Local tree-sitter differential (`tools/oracle_diff.py <halmi> go` over
the TensorAST `corpora` tree, 93 files, not vendored; private oracle binary,
not in this repository, so historical and UNREPRODUCIBLE from this repo):
257/257 exact spans on the 91 well-formed files; 20 body-less declarations
excluded by contract; the 2 malformed fixtures are the only differences.

## No-claim boundaries

- `select` counts as a branch; individual `case` clauses also count.
- Generic type parameters are not distinguished from parameters in
  signatures (rare in practice; io_params may overcount on generic
  functions).
- Unnamed parameter types (`func f(int, string)`) may count the first type
  as a parameter name; named parameters (the common case) are exact.
- Real-corpus validation (2026-11-20): 2,363 files / 33,375 functions,
  0 degraded files, zero panics -- historical, UNREPRODUCIBLE (corpus and
  harness not recorded in this repo).
- Malformed signatures (a missing brace or parenthesis) yield no error
  entry: the scanner is not a parser, so a file with a syntax error can
  report zero functions with exit status 0.
