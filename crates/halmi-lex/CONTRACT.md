# CONTRACT.md -- halmi-lex

## Purpose and layer

`halmi-lex` is the shared token and collection layer between the source
adapters and `halmi-core`. It defines token types, language-specific
classification tables, metric collection, and C-family scanning utilities. It
contains no external parser, tree-sitter dependency, global state, IO, or
policy for ranking priorities.

`halmi-core` owns formulas and public report semantics. Each adapter owns its
own scanner, declaration detection, qualification, error messages, and token
occlusion rules. halmi-lex deliberately does not claim that one generic token
stream is a syntax tree.

## Public types

### `Tok`

The shared token variants are:

- `Ident(text)`
- `Number(text)`
- `Str(text)`
- `Punct(text)`
- `Keyword(text)`
- `Newline`

Adapters map native tokens into these categories before applying
`MetricsCollector`. Text is borrowed from the adapter's source buffer, not
copied into a long-lived parse tree.

### `LangSpec`

`LangSpec` supplies the adapter's measurement definition:

- `decl_op`: one declaration operator kind inserted once at construction and
  once after every `finish` or `reset`.
- `keyword_operators`: keywords counted as operators by the shared lexical-map
  rule.
- `cc_keywords`: keywords that each increment cyclomatic complexity.
- `separator_ops`: punctuators that are separators rather than operators.
- `cc_ops`: punctuation that increments cyclomatic complexity.
- `loc_mode`: `Logical` or `Physical` code-line counting.

`PAREN_SEPARATORS` is the supplied common parenthesis, bracket, brace, comma,
semicolon, and colon separator set. It is not a universal statement that these
characters are never meaningful in other languages.

### Logical and physical LOC

- `Logical` increments a line when `Newline` terminates a line with code state.
  It also flushes that state at `finish`. Backslash and bracket continuations
  therefore require adapter handling in their scanners.
- `Physical` counts distinct 1-based lines with an identifier, number, string,
  keyword, punctuator, or block token. Calls to `open_block` and `close_block`
  count only once per source line.

`LocMode` participates in the MI input definition. An adapter that reports
physical LOC must say so in its own contract. Physical lines and Visual Studio
MSIL instructions are not interchangeable quantities.

### `MetricsCollector`

`MetricsCollector::new(spec, base_depth)` inserts the declaration operator and
starts with `cc = 1`, the provided nesting base, and empty operator maps.

The collector supports:

- `token`: logical- and physical-line signals, operator/operand counting, and
  cyclomatic increments based on the current `LangSpec`.
- `call(is_method, extends)`: records a call operator and updates the current
  chain length. Dotted calls start at one; receivers continuing an existing
  chain extend it; plain calls reset it; max-chain monotonically increases.
- `index`: records a subscript/index operator.
- `open_block` / `close_block`: adjusts block depth by one and tracks maximum
  nesting. Closing depth is saturating, not negative.
- `set_depth`: sets an indentation-based current depth and records its maximum.
- `current_depth`: returns that depth for adapters tracking indentation.
- `reset`: explicit manual reset with retained allocation capacity.

#### Function completion and reuse

`finish(io_params)` returns a `MetricsOutcome` with four raw Halstead counts,
cyclomatic complexity, LOC, maximum nesting, and maximum chain length. It then
resets per-function state for the next function.

`io_params` is the adapter-determined number of unique input/output parameters.
It affects potential volume in halmi-core, not the ordinary volume and effort
computed from this collector. Interpretation of constructors, receivers,
variadics, `void`, and optional results is necessarily adapter-specific.

After `finish` the collector has:

- cleared operator/operand occurrence maps without discarding vector capacity;
- reset cyclomatic complexity to 1 and reset chain state;
- reset logical/physical LOC state;
- restored `depth` and `max_nesting` to `base_depth` from construction;
- inserted the declaration operator for the next function exactly once.

Residual function-body depth must not leak into a subsequent function. A
per-file collector reused this way is not the same as constructing a collector
fresh for each function: reuse must preserve language-span ownership and all
per-function outputs.

### `density`

`density(map, start_line, end_line)` returns the fraction of comment-bearing
physically numbered 1-based lines in the inclusive span. It clamps the start
to zero and the end to the map length and returns 0 on an empty or reversed
range. Comment syntax and comment recognition are the caller's responsibility;
this helper does not classify text inside string literals as comments.

### `cfamily`

The C-family toolkit includes comment/string/raw-string scanners, numeric
scanning, UTF-8 lead-byte length, and the ordered `punct_len` punctuator dispatch.
Its longest-match ordering is part of its semantics. A seemingly concise rewrite
can silently lose operators such as `=>`, `>>>=`, ellipsis variants, `??=`, or
context-sensitive dots. Changes here require all available differential modes,
not just a handful of C examples.

## Error model

The collector itself returns plain data, not `Result`, and does not perform IO
or throw language-parse errors. Scanner utilities return `Result` with start
or line metadata where applicable for unterminated text and malformed
constructs. Structured per-file failures are the adapter's responsibility.
UTF-8 fallback and checked-slice behavior must keep malformed input from
crashing an adapter.

## Invariants and determinism

`MetricsCollector` is a deterministic mutable state machine for valid native
token streams. Identical tokens and base configuration produce identical
outcomes. Its construction, token calls, and finish order matter; cross-function
state must not depend on hash-map iteration order, wall clock, or global state.
The implementation uses vectors and linear searches for operator/operand maps,
so output never depends on randomized hash seeds.

`reset` restores the same base nesting kind chosen at construction. Tests cover
both ordinary use and a nonzero base. This prevents a later function from
inheriting another function's residual structure, which would affect nesting
and joint priority but not erase the underlying Halstead operator counts.

No cancellation API is provided: work is synchronous and bounded by caller
input. Unsafe code is not used at crate level.

## Conformance

`src/lib.rs` carries a keyword-table test and `tests/lex.rs` adds 13 focused
integration tests, 14 total. They cover declaration accounting, separators,
branches, method chains, both LOC modes, comment density, punctuator order,
numbers, quoted and raw text, nesting, and constructor-base restoration.

Run:

```bash
cargo test -p halmi-lex --test lex
```

The shared collector also gains indirect evidence from integration suites of
every adapter and from deterministic corpus reports. Those do not independently
prove that a language adapter's operator/operand classification is correct.

## No-claim boundaries

- This layer is not a full parser, type checker, or source-of-truth on what is
  an operator in every language. Halstead classification remains a documented
  adapter convention.
- A body span's meaning and exclusions are defined by its adapter. Hidden
  declarations, macros, closures, and nested functions are not all treated the
  same way across languages.
- Method chains are shallow lexical heuristics, not proof of fluent API design
  or runtime dispatch structure.
- The physical-line and logical-line measurements can disagree. Totaling one
  semantic kind of LOC across adapters is not valid.
- Potential-volume I/O parameters provided by an adapter are approximations,
  not a proof of the true conceptual minimum operands.
- Conditional preprocessor output is a token-level model, not macro evaluation
  or a C/C++ compiler's actual selected branches.
- Semantics of `Punct` are language-specific. The shared collector must not
  infer additional operators beyond its supplied `LangSpec` and the explicit
  `call`, `index`, and `attr` conventions without a new reviewed contract.
