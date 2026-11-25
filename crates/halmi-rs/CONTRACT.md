# CONTRACT.md -- halmi-rs

## Purpose and layer

The Rust language adapter for halmi. Middle layer: walks a `syn` AST exactly
once per function body and produces `halmi_core::FunctionReport` values with
Halstead counts, cyclomatic complexity, logical LOC, nesting depth, precise
line spans, and comment density. Depends only on `halmi-core` + `syn` /
`proc-macro2` / `quote`. The CLI (and any embedder) calls `analyze_source`
and then `halmi_core::ranking::finalize`.

## Public types and semantics

- `RustAdapter` -- implements `halmi_core::adapter::LanguageAdapter`; the
  reference adapter. A new language is a new crate with the same contract
  (pure, never panics, one pass); everything downstream is shared.
- `analyze_source(path: &str, src: &str) -> SourceAnalysis` -- convenience
  wrapper over `RustAdapter.analyze`; `path` is echoed verbatim into every
  report.
- `SourceAnalysis` is re-exported from `halmi-core` (single definition;
  language-neutral). `functions` are in source order, unranked
  (`priority == 0`, empty `reason`).
- `analyze_file` was removed: file reading is IO and belongs at the caller
  edge (the CLI reads files itself).

### Which functions are analyzed

Free functions (`fn`), inherent impl methods (named `Type::method`), trait
default methods (named `Trait::method`), and functions inside nested inline
modules. Excluded: trait method signatures without bodies, nested items
inside a body (a nested `fn` is neither reported nor counted in the parent),
and anything inside macro token streams (each macro invocation is a single
operator, e.g. `macro:println`).

### Span semantics

`start_line..=end_line` is 1-based inclusive, from the first token of the
signature through the last token of the body. Attributes, doc comments, and
visibility are excluded; `async` is part of the signature.

### Metric definitions (token-faithful, see `src/classify.rs` header)

- Operators: arithmetic/logical/comparison/bitwise/assignment operators,
  unary `!`/`-`/`*`, ranges, `?`, `as`, field access, method calls, calls,
  indexing, struct/array/repeat literals, `fn` itself, control keywords
  (`if`, `match`, `while`, `for`, `loop`, `return`, `break`, `continue`,
  `await`, `let`, `unsafe`, `async`, `yield`), closures, and per-macro
  operators. The function's own signature (name, params, return type) is
  excluded.
- Operands: identifiers (locals, fields, function/method names, path
  segments, tuple indices) and literals, keyed by token text; `self` counts.
- Cyclomatic complexity: 1 + one per `if` (incl. `else if`), `while`,
  `while let`, `for`, `loop`, match arm, `&&`, `||`, `?`.
- `io_params` (eta2*): distinct parameter names in the signature (`self`
  included) plus one for a non-unit return type; drives V*.
- Method chains: `method_chain` is the longest run of consecutive method
  calls on one expression spine; receiver sub-expressions extend the chain,
  arguments start a fresh one. Long chains are idiomatic (iterator/builder
  style) and get their own reason clause upstream instead of the generic
  "dense expressions" one.
- Logical LOC: statement count including statements inside nested blocks,
  plus one per match arm; nested items do not count.
- Nesting depth: max depth of nested control-flow constructs; the function
  body is 0, `else if` chains stay flat, closures do not add depth.
- Comment density: always `Some(fraction)` for spans with at least one line
  (this adapter counts comments); the scanner is literal-aware (plain/raw/
  byte strings, char vs lifetime).

## Invariants

1. Never panics on any input: syntax errors return `functions: []` plus an
   `error` message (`tests/analysis.rs::parse_errors_degrade...`).
2. Trivial bodies (empty, comment-only, or macro-only) yield non-meaningful
   Halstead metrics and MI = 100 on all variants; a comment-only body is
   trivial because it has no statements and no operands
   (`tests/analysis.rs::trivial_and_macro_only_bodies`,
   `tests/comments.rs`).
3. Analysis is idempotent: identical source produces identical reports
   (`tests/analysis.rs::full_pipeline_ranks_and_analysis_is_idempotent`).
4. Counts are exact and hand-verified for reference snippets (add, fib,
   try/cast, impl methods) (`tests/analysis.rs`).
5. Comment scanning ignores comment markers inside string/char literals and
   respects lifetimes, raw strings with hashes, and nested block comments;
   densities are pinned through the SEI comment term end-to-end
   (`tests/comments.rs`).

## Error model

A single optional `error: Option<String>` per file: either the `syn` parse
message or the IO error text. There are no panics, no `Result`s, and no
error taxonomy -- callers decide how to aggregate (the CLI collects them
into `summary.errors`).

## Determinism class

Deterministic for a given source string and `syn` version: a single AST
walk, report order equals source order, `HashMap` is used only for counting
and is immediately reduced to `HalsteadCounts`. No clocks, randomness, IO
(beyond `analyze_file`), or global state.

## Cancellation behavior

Synchronous CPU work bounded by file size. Per-file isolation: one bad file
never corrupts another file's analysis. Nothing to cancel; no partial state.

## Unsafe boundary

None. `#![deny(unsafe_code)]` is set at the crate root.

## Feature flags

None.

## Conformance tests

- `tests/analysis.rs` -- spans, qualified names, exact Halstead counts,
  CC/LOC/nesting rules, exclusions, degradation, full pipeline +
  idempotence.
- `tests/comments.rs` -- the literal-aware comment scanner, end-to-end
  through the SEI term.
- `tests/common/mod.rs` -- shared `analyze`/`find` fixtures.

Mutation-verified: dropping the match-arm CC increment and disabling the
raw-string hash guard both fail the suite. The scanner's literal taxonomy
(byte-char literals included) is pinned by tests written red-first after a
real-corpus crash (ADR-0014).

## No-claim boundaries

- Macro bodies are opaque: one operator per invocation, no inner metrics.
- Code generated by proc macros at compile time is invisible (source-level
  analysis only).
- Operator/operand classification is token-faithful, not semantically typed
  (all method calls are one operator kind; `+` for ints and floats are the
  same operator).
- Spans come from `proc-macro2` fallback mode: line-accurate, no column or
  byte-offset guarantees exported.
- Only Rust is analyzed; other languages need their own adapter crate.
