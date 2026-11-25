# CONTRACT.md -- halmi-python

## Purpose and layer

The Python adapter for halmi: a hand-written lexer (no parser dependency)
feeding one structure pass that yields `RawFunction`s. Implements the
shared `halmi_core::adapter::LanguageAdapter` port: pure, never panics,
one pass, degraded files become error entries. Everything downstream
(metrics, banding, ranking, delta, gates, JSON/SARIF) is shared.

## Public API

- `PythonAdapter` -- `language() == "Python"`, `analyze(path, src)`.

## Classification (token-faithful)

- Operators: symbol operators (`+ - * ** / // % @ = += ... << >> & | ^ ~
  < > <= >= == !=`), attribute access `.`, invocation `call`, subscript
  `index`, and the keywords `def if elif for while return break continue
  lambda yield raise with and or not in is`. Separators and brackets
  (`, ; : ( ) [ ] { }`) are excluded. Every analyzed function carries
  exactly one `def` declaration operator (mirrors the Rust adapter's
  `fn`); nested defs are separate functions, not part of the parent.
- Operands: identifiers that are not keywords, numbers, and string
  literals (an f-string is one operand; interpolations are not lexed).
- Cyclomatic complexity: 1 + one per `if`, `elif`, `for`, `while`,
  `except`, `and`, `or` (ternary `if` counts via `if`).
- Logical LOC: logical lines (backslash continuations and open brackets
  join lines); comments and blanks excluded.
- Nesting: indentation depth relative to the function body (body = 1;
  each deeper indented block adds one).
- Nested `def`/`class` inside a body: excluded from the parent's counts
  and not reported, matching the Rust adapter.
- Spans: the `def` line through the last body line; decorators and
  trailing blanks excluded.
- `io_params`: distinct parameter names plus one for a `->` return
  annotation. `method_chain`: consecutive called attribute accesses
  (`x.m1().m2()`), receiver spine only.
- Comment density: `#` lines in the span (always `Some`).

## Invariants

1. Never panics: unterminated strings, inconsistent dedents, and stray
   characters degrade the whole file to `error` with `functions: []`.
   Unicode identifiers and non-ASCII symbols are lexed safely (byte
   slicing is char-boundary-guarded).
2. Same pipeline, same contract: `reports_from_raw` -> `finalize` works
   unchanged; output is deterministic and idempotent.
3. Soft keywords (`match`, `case`) are valid identifiers (`def case(...)`
   works); match-statement arm CC is therefore not counted (no-claim).

## Error model / determinism / cancellation / unsafe / features

Single optional `error` string per file (lex errors only; no IO in this
adapter). Deterministic single pass, report order = source order.
Synchronous, bounded by input. `#![deny(unsafe_code)]`. No feature flags.

## Conformance tests

`src/lib.rs` has 1 keyword-table test and `tests/python.rs` has 18 focused
integration tests, 19 total: spans/names/counts, shared-pipeline determinism,
literal-aware comments (triple-quoted, f-strings), bracket continuations,
nested-def exclusion, unicode safety, soft keywords, same-line and indented
suites, and degradation.

## No-claim boundaries

- f-string interpolations are not lexed (the literal is one operand);
  the `f`/`r`/`b`/`u` prefix is part of the literal, not an operand.
- Match-statement arms do not add CC (soft-keyword handling).
- Tabs count as 8-column stops for indentation.
- Real-corpus validation (2026-11-20): 18,942 files / 233,793 functions,
  12 degraded files (0.005%), zero panics.
