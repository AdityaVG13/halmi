# Contributing to halmi

halmi welcomes bug reports, questions, documentation improvements, performance
experiments, new tests, and implementation changes. Open an issue, suggest an idea, fork the project, make a branch, or open a
pull request.

This guide helps contributors produce evidence-backed changes and useful
review discussions. It is intentionally separate from any private machine
configuration or internal agent state.

## Quick start

1. Read [README.md](README.md) for the product surface and examples.
2. Read [docs/SCIENCE.md](docs/SCIENCE.md) for formula provenance, metric
   definitions, and interpretation limits.
3. Read the `CONTRACT.md` for any crate you change. Every adapter has one:
   classification, spans, error behavior, invariants, and no-claim boundaries.
4. Try the binary and its focused tests on a clean checkout:

   ```bash
   cargo build --release -p halmi-cli
   cargo test -p halmi-cli --test cli
   cargo test -p halmi-core --test formulas
   cargo test -p halmi-lex --test lex
   ```

Rust **1.100.0** is pinned in `rust-toolchain.toml`. It is a stable toolchain,
not a required nightly channel. Use that exact version for reproducible
benchmark comparisons.

## Where discussions happen

### Issues

Open an issue for any of these:

- A crash, panic, wrong report, misclassification, or wrong span.
- A new syntax form or language feature that should be supported.
- Confusing documentation, unclear evidence, or misleading metric semantics.
- A dependency, boundary, generated-report, or Microsoft JSON/SARIF concern.
- A reproducible performance regression or a reproducibly better approach.
- A proposal for a new adapter, more useful gates, or new evidence workflow.

Small issues are welcome. You do not have to prove an idea deserves to exist
before asking a question. Old misses should stay visible, since a superseded
result is not equivalent to a solved bug.

### Pull requests

Open a pull request when you have a contribution to review. It can be one fix,
one test, a documentation repair, an artifact reproducibility repair, or a
focused implementation. Drafts and small experiments are also useful if the
uncertainty is stated clearly.

Do not wait for permission to start writing code. For unusually large or
architecturally sensitive changes, an early issue can save review time, but
this is a coordination recommendation, not an approval gate.

### Security-sensitive findings

Do not publish private corpus material, tokens, passwords, customer data, or
exploitation details in a normal issue or pull request. Use the repository's
private security advisory channel when it is available, or ask the maintainer
for a private contact rather than disclosing sensitive artifacts publicly.

## What makes a good issue

Include what you can verify, not what would make the report sound dramatic.
A useful report usually has:

- The halmi command and binary artifact, if you changed or built it yourself.
- `halmi --version`, `rustc --version`, OS/CPU family, and whether the fixture
  was built with stable, a different pinned configuration, or a custom wrapper.
- The smallest relevant source or fixture, source path, function name, span,
  and language/extension.
- The expected versus actual behavior, including relevant JSON, JSON errors,
  table text, or a stack trace with `RUST_BACKTRACE=1`.
- A recipe for reproducing it: flags, baseline artifacts, file names, corpora,
  machine context, and where an oracle or fixture differs if that matters.
- What you already tested, what contradicted the initial hypothesis, and what
  remains unknown.

For metric questions, name the metric and semantic question separately from
style preference. Higher CC, effort, volume, or lower MI are not automatically
bugs. There may be a real parsing issue, a wrong classification, a natural
tradeoff, a rollup artifact from more functions, or a legitimate reason not to
change anything.

For benchmarks, include full commands, corpus selection, binary/toolchain,
host architecture, load conditions, repetitions, and residual counts. A raw
local observation without this context is not a reproducible performance claim.

## Choose a useful next task

### A minimal contribution

Examples of a useful first contribution:

- A discriminating regression test for one malformed literal, declaration, span,
  or state reset.
- An error with an incorrect line number or a malformed diagnostic.
- An incomplete adapter contract or confusing no-claim boundary.
- A reproducible invalid JSON number, surrogate pair, or empty-result/report
  handling case.
- A similarly parsed code example that exposes a bad operator/operand
  classification without changing a parser's broader semantics.
- A test fixture that makes a prior implementation failure observable before a
  small code correction is attempted.

Focus on problems people can observe, not on re-ranking your own source files
merely because halmi selects them.

### New adapters or syntax support

At minimum, contribute the adapter behavior plus its tests plus its
`CONTRACT.md`. Include:

- Extension routing and expected function ownership/spans.
- Native troublesome tokens, nested blocks, closures, strings/comments, numbers,
  identifiers, interpolation, modifiers, generic/receiver forms, and attributes.
- Operator/operand/separator definitions and whether LOC is physical or logical.
- Comments and comment-density handling.
- Return/result and constructor semantics:
  `io_params`, receiver rules, return annotations, `void`, `never`, or unit.
- Local corpus residual counts and explicit no-claim boundaries for shapes it
  cannot support yet.

Prefer the existing hand-lexing pattern and `MetricsCollector`. Adding a parser
framework is a product architectural choice, not a requirement for every syntax
fix. `halmi-rs` intentionally uses `syn`, `proc-macro2`, and `quote`; other
adapters use dependency-free shared machinery. No tree-sitter in the product
or in this repository: the span differential's oracle is a private developer
tool that is not shipped, so the tree-sitter figures in the crate contracts
are historical local observations, not something a clone can reproduce.

### Performance work

Measure what actually dominates cost. Profile end-to-end CPU time and report
commands, number of functions, errors, and exact corpora. Preserve metrics and
spans unless a behavior change is the purpose of the change.

A useful performance PR says why it is faster, where the cost went, and what
evidence rules out accidentally dropping identical analysis work. Include
initialization, cache, output emission, and thread handling when relevant.
Do not compare only warm-cache best cases.

## Workflow for code and docs

### 1. Create a branch in your fork

```bash
git checkout -b <topic>-<short-description>
```

Examples: `python-one-line-suites`, `reset-nesting`, `readme-examples`,
`json-surrogate-check`, `kotlin-nested-comments`.

The `main` branch belongs to the upstream repository. A PR may come from a
fork branch, an upstream feature branch when authorized, or a small local checkout.
Use your normal Git workflow. You do not need owner approval to choose a branch
name in your own fork.

### 2. Use focused regression evidence

For a semantic defect:

1. **Reproduce first.** Use the actual program or oracle where practical rather
   than a description of something found in one source file.
2. **Write a discriminating test.** Make sure it fails for the defect, not
   because the fixture is malformed for unrelated reasons.
3. Make the smallest sound implementation change.
4. Run the same test and its relevant integration paths again.
5. Preserve the artifact or transcript showing the failing and passing states.

For pure refactoring, require full report equality on the same fixed corpus,
with identical source root and selected outputs. For intentional behavior or
schema changes, name the new value and state why it is now correct.

A test should fail on an actual bug, not merely restate the existing program
output. Avoid tests for incidental allocation counts, brittle log strings,
or snapshot changes that say nothing about intent. Some refactors need no
additional unit tests when an existing oracle clearly pins unchanged behavior;
say which one protects it.

Don't delete fixtures or rename them into an oracle-error bucket without
documenting the grounds. Residuals demonstrate limits, not necessarily product
defects, and should remain distinguishable from genuine failures.

### 3. Run focused checks

Choose commands for everything the change actually touches. Useful examples:

```bash
# CLI workflows and CLI schema
cargo test -p halmi-cli --test cli

# Formulas, JSON parser, reports, and MI semantics
cargo test -p halmi-core --test formulas
cargo test -p halmi-core --test json
cargo test -p halmi-core --test report

# Shared token collection and constructor-base reset
cargo test -p halmi-lex --test lex

# Example discriminating adapter regressions
cargo test -p halmi-python --test python one_line_suites_and_sequential_defs_keep_their_boundaries
cargo test -p halmi-ruby --test ruby iterator_do_inside_a_loop_remains_a_block
cargo test -p halmi-kotlin --test kotlin nested_block_comments_do_not_expose_declarations

# Formatting, API docs, comments, and dependency advisories
cargo fmt --check -p halmi-cli
cargo doc --no-deps -p halmi-core
cargo audit --no-fetch
python3 tools/comment_lint.py
```

For a public contribution, these named local lanes are the interface, not a
private remote worker or tool environment. Run what you can access and report
anything unavailable with its exact failure text rather than replacing it with
a green result from another machine.

After changing source or toolchain state, invoke the binary you just built:

```bash
cargo build --release -p halmi-cli
./target/release/halmi --json --top 10 /path/to/example.rs
```

Do not count an older binary or test transcript as proof that a workspace whose
code just changed has the same behavior. Refresh the artifact and record it.

### 4. Keep public types and JSON output understandable

`FunctionReport` is the common unit of function analysis, and by-file rows must
not be confused with a whole-file Halstead equation without a sound aggregator:
`max_difficulty`, total effort, mean/median MI, total LOC, and highest-priority

`worst` have precise meanings. The operation of adding functions can affect
those means, medians, gates, and explanations.

Preserve deterministic output ordering, backward-compatible parsing for unknown
additional fields, and structured errors for malformed or nonfinite input. The
reader cannot fix bad measurements or infer missing classifications for you.

When APIs, fields, counts, or meanings change, update all coupled surfaces:

- relevant `CONTRACT.md` files and code comments;
- parser and CLI fixtures;
- [README.md](README.md) if user-visible behavior changes;
- [docs/SCIENCE.md](docs/SCIENCE.md) for provenance and interpretation;
- [docs/BENCHMARKS.md](docs/BENCHMARKS.md) whenever a published example number
  changes or a new corpus measurement supersedes an earlier one.

Do not add a field just to compensate for an ambiguous upstream definition,
especially by adding near-duplicate summary stats.

### 5. Review and respond to feedback

A focused PR should describe:

- The user-visible change and why it is useful.
- Files/crates touched and the code path addressed.
- Evidence of before/after behavior, full-corpus comparison, or intentional
  delta changes.
- Exact commands used and their exit status.
- Known limits, residual syntax, and anything unmeasured or non-reproducible.
- The relevant new or refreshed `CONTRACT.md` sections.

Small corrections are normal. If a reproducer no longer matches the actual
path, adjust the fixture and explain the correction. A red case can reveal a
second defect in a receiver, scope boundary, or metric definition. That is not
a reason to keep patching elaborate new systems around the same spot; stop,
state the cause, and choose the smaller root repair.

Feedback is about whether a change is correct and useful, not whether an author
is experienced or has commit access. Reimplementation without evidence is not
preferable to a well-tested small fix.

## Documentation standards

- Use plain language and no em dashes.
- Preserve independent sources, confidence boundaries, and observation status:
  known test evidence, adapter-specific rules, heuristics, historical outputs,
  missing primary citations, and explicit unreproducible observations.
- Treat function spans and operator/operand classification as separate kinds
  of evidence. Tree-sitter can agree on spans while an adapter still classifies
  tokens differently.
- Do not invent collected citations, performance figures, or new truth claims
  such as "fuzzer-tested", "scientifically proven complexity", or "100% support".
- Update internal formula/provenance links without claiming a bibliographic
  record equals a paper read in full. [SCIENCE.md](docs/SCIENCE.md) records
  which sources were actually inspected and to what extent.
- Leave residual datasets and negative controls discoverable. Reintroducing a
  private corpus is not acceptable proof of a missing or newly verified fix.

[docs/BENCHMARKS.md](docs/BENCHMARKS.md) illustrates the expected publication
style. It reports function counts and actual host conditions, not vague "fast"
claims or invented universal SLOs. Data marked **UNREPRODUCIBLE** should name the
missing pinned environment and corpus so a reviewer cannot mistake it for
reproducible CI evidence.

## The contributor path in practice

1. Find and reproduce the issue or state the proposal.
2. Create a fork and branch in your normal Git hosting setup.
3. Read the relevant contract and check the focused test lane.
4. Implement the smallest sound fix and relevant regression coverage.
5. Preserve evidence on actual artifacts; do not trust an old stale executable.
6. Update documentation and tests with the new behavior.
7. Run the focused checks, inspect generated files, and open a PR early if the
   approach would benefit from feedback.
8. Work through review results honestly, including fresh parser, metric, or
   documentation discoveries.

For the bounded agent-assisted version of that workflow, see
[`skills/halmi/SKILL.md`](skills/halmi/SKILL.md). It preserves a baseline while
allowing up to three focused repair attempts and reports both improvements and
mixed MI tradeoffs. The workflow is optional for contributors; it is not a proxy
for maintainer reviews, meaningful tests, or scientific evaluation.

## Release posture

Release actions are performed by designated maintainers. Contributors are not
asked to bypass repository permissions to make a tag, publish a package, or
deploy to production. If a change should be considered for a release, call it
out with evidence in the PR or issue instead of independently publishing it.

That constraint does not change your ability to file an issue, fork the code,
create a branch in your fork, build, test, or submit a pull request. For an
unreleased contribution, there is still no owner approval barrier.

## License

By contributing, you allow your contribution to be used and distributed under
the project’s [MIT license](LICENSE). Keep MIT-compatible third-party attribution

where required by imported code, tests, or derived documentation. Do not include
private repositories, credentials, or unlicensed source material as fixtures.

Thanks for contributing to a clearer, better measurement tool
