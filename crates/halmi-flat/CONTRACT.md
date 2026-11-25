# CONTRACT.md -- halmi-flat

## Purpose and layer

The minimal reference language adapter for halmi: a ~150-line crate that
proves the `halmi_core::adapter::LanguageAdapter` port. A new language is a
new crate with this shape; everything downstream (metrics, banding,
ranking, delta, gates, JSON) is shared with the Rust adapter. It analyzes
"flat-script", a line-based toy language defined entirely in this file.

## Public API

- `FlatAdapter` -- implements `LanguageAdapter` (`language() == "flat-script"`).
- Nothing else; use the trait (or `halmi_rs::analyze_source`-style helpers
  in real adapters).

## flat-script grammar

- A function starts at a `def NAME:` line; its body is the following run of
  lines with strictly greater indentation (blank lines tolerated, trailing
  blanks excluded from the span).
- Operators: the symbol tokens `+ - * / % < > =`. Operands: word tokens
  other than `def`/`if`. Counts key by token text.
- Cyclomatic complexity: 1 + occurrences of the word `if`.
- Logical LOC: non-comment body lines. Nesting: always 0 (flat language).
- Comment density: `#`-prefixed lines in the span (always `Some`).
- `method_chain`: always 0 (the language has no method calls).

## Invariants

1. Never panics: malformed structure (a top-level line that is not
   `def`/comment, a bad `def` form, a bad name) degrades the whole file to
   `error` with `functions: []`.
2. Same pipeline, same contract: `reports_from_raw` -> `finalize` works
   unchanged; output is deterministic and idempotent (pinned in
   `tests/flat.rs`).
3. Zero external dependencies beyond `halmi-core`.

## Error model / determinism / cancellation / unsafe / features

Single optional `error` string per file (parse errors only; no IO in this
adapter). Deterministic single pass over lines, report order = source
order. Synchronous, bounded by input. `#![deny(unsafe_code)]`. No feature
flags.

## Conformance tests

`tests/flat.rs` -- spans/counts exactness, shared-pipeline ranking +
determinism, comment density through the SEI term, degradation. The chain
and span mutations (counter disabled, off-by-one span) are caught by the
suite.

## No-claim boundaries

flat-script exists to be a template, not to be useful: no type system, no
expressions spanning lines, no nesting. Its Halstead numbers are
token-level only.
