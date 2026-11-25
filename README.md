# halmi

**Measured refactoring guidance rather than coding by intuition.**

halmi analyzes source functions with Halstead metrics and Maintainability Index
variants. It emits per-function measurements, per-file rollups, and useful
inspection targets through a CLI, SARIF, and a Rust library API.

Use its reports to **guide** inspection, not to delete mechanisms just to lower
a score. Priority is a heuristic, not a universal quality verdict or a defect
probability.

| Looking for... | Go here |
|---|---|
| Install, build, and API | [Quick start](#quick-start) |
| Metrics and caveats | [Measurement model](#measurement-model) |
| CLI use | [Workflows](#workflows) |
| `/halmi` agent workflow | [`skills/halmi/SKILL.md`](skills/halmi/SKILL.md) |
| Formula provenance and scientific limits | [docs/SCIENCE.md](docs/SCIENCE.md) |
| Output measurements and provenance metadata | [docs/BENCHMARKS.md](docs/BENCHMARKS.md) |
| Fork-and-PR contribution | [CONTRIBUTING.md](CONTRIBUTING.md) |
| Adapter-specific behavior | Per-crate `CONTRACT.md` files |

## Why halmi exists

Complexity metrics often answer only one question: how branch-heavy is this
function? halmi also asks whether its operator and operand vocabulary is dense,
whether refactoring removed work or moved it somewhere else, and whether MI
means and medians agree with the new headline complexity figure.

The point of halmi is not to certify “clean” code. It is to make reviewable
statements about measurable input: which functions are worth a look, whether a
change was behavior-preserving, and where judgment and scientific evidence
still have to be supplied.

Adapters document precisely what they count. Arithmetic is exact and tested;
interpretation remains constrained by conventions and real-world variability.

## Quick start

### Requirements

- **Rust 1.100.0**, pinned in `rust-toolchain.toml`
- No tree-sitter in the product or in this repository. The tree-sitter span
  differential uses a private developer tool that is not shipped.

### Build, install, and test

```bash
cargo build --release -p halmi-cli
cargo test -p halmi-cli --test cli
cargo test -p halmi-core --test formulas
cargo test -p halmi-lex --test lex
```

The resulting executable is `target/release/halmi`. Install it from this checkout:

```bash
cargo install --path crates/halmi-cli --locked
halmi --version
```

Test it against your own source directory:

```bash
halmi --json --top 5 /path/to/source
```

For people, ordered human-readable tables, or CI ingestion:

```bash
halmi /path/to/source --format table
halmi /path/to/source --format sarif > results.sarif
```

There are **15 supported languages**. See
[Supported languages and contracts](#supported-languages-and-contracts).

### One complete example

Consider this ordinary Rust function:

```rust
pub fn add(a: u32, b: u32) -> u32 {
    a + b
}
```

Under the Rust adapter's accounting:

- `fn` and `+` are the distinct operators: `eta1 = 2` and `N1 = 2`.
- `a` and `b` are the distinct operands: `eta2 = 2` and `N2 = 2`.

Its measured values are therefore also directly hand-checkable:

```text
eta = 4
N   = 4
V   = 4 * log2(4) = 8
D   = (2 / 2) * (2 / 2) = 1
E   = 1 * 8 = 8
CC  = 1
LOC = 1
```

Its report gives `volume = 8`, `difficulty = 1`, `effort = 8`, `loc = 1`,
`cyclomatic = 1`, and `mi_vs = 93.542` after rounding to three decimals.

Validate the hand-derived result yourself with:

```bash
cargo test -p halmi-core --test formulas
cargo test -p halmi-rs --test analysis simple_function_counts_exactly
```

That distinguishes arithmetic conformance from a claim that a particular
classification is universally correct.

## Measurement model

### Halstead suite

Adapters collect four function-level inputs:

- `eta1`: distinct operators
- `eta2`: distinct operands
- `N1`: total operator occurrences
- `N2`: total operand occurrences

From these, halmi computes volume `V`, difficulty `D`, and effort `E`, plus
length, level, estimated length, purity ratio, potential volume, and alternative
length forms as exposed in the API contract.

`estimated_bugs` is `V / 3000` and `time_seconds` is `E / 18`. Both are relative
model conventions with uncertain empirical support, not predicted defect
counts or developer stopwatch readings. Potential volume uses
adapter-approximated I/O parameters and is likewise a bounded heuristic.

### Maintainability Index

```text
MI_original = 171 - 5.2 * ln(volume) - 0.23 * CC - 16.2 * ln(LOC)
MI_VS       = max(0, MI_original * 100 / 171)
```

Lower MI-VS means more scrutiny. MI-VS clamps negative MI-original results to 0.

The optional comment-density variant is:

```text
MI_SEI = MI_original + 50 * sin(sqrt(2.4 * clamped_comment_density))
```

It is not normalized to the Microsoft visual-studio range and is not used for
priority. Tiny function semantics are original and MI-VS 100; MI-SEI is 100
when density is available and absent otherwise.

Halmi's MI-Bands are:

```text
high:     >= 85
moderate: >= 65 and < 85
low:      < 65
```

These differ from Microsoft’s green (>= 20), yellow (10-19), and red (0-9)
thresholds. Do not confuse the two scales.

### Cyclomatic complexity and related signals

`cyclomatic` begins with 1 in each function. Adapter-defined control keywords
and branch operators increment it. LOC may mean logical statements or physical
lines depending on the adapter contract. Nesting is actual brace/indent depth.
Method chains are shallow source-structure signals rather than type-level call
spines.

### Joint priority, an inspection heuristic

```text
priority = (effort / max(median_effort, 1.0))
         * (100 - mi_vs)
         * (1 + 0.15 * max(0, nesting - 2))
         * (1 + 0.2 * max(0, ln(loc)))
```

Priority is repository-relative and **uncalibrated** for defect prediction,
impact, or implementation time. New helper functions change the distribution
being compared. Do not treat an improvement in priority alone as unqualified
maintainability improvement.

## Workflows

| Task | Command |
|---|---|
| Analyze an entire tree | `halmi src/` |
| Show the first 5 joint-priority findings | `halmi src/ --top 5` |
| Use a table instead of JSON | `halmi src/ --format table` |
| Emit SARIF 2.1.0 for CI | `halmi src/ --format sarif > results.sarif` |
| Show file-level Halstead difficulty order | `halmi src/ --by-file --sort difficulty` |
| Order by relative effort | `halmi src/ --sort effort` |
| Order by MI, worst first | `halmi src/ --sort mi` |
| Exclude paths (repeatable) | `halmi src/ --exclude 'vendor/*' --exclude '*generated*'` |
| Disable table color when necessary | `halmi src/ --format table --color never` |

`--top` limits results after metrics and gates. It does not change the corpus,
computed medians, enabled gate evaluations, or error summaries.

### Baselines and deltas

```bash
halmi src/ --json --top 100000 > /tmp/before.json
# Refactor or edit selected source files
halmi src/ --baseline /tmp/before.json
halmi src/ --baseline /tmp/before.json --changed-only
```

`FunctionReport` records are paired by name and path, with a root-suffix heuristic
for relocated roots. Delta labels cover worsened priority (> 0.5 increase),
improved priority (< 0.5 decrease), new functions, and secondary MI regression
when current MI is below 50, baseline drops, and effort increases.

Those labels are evidence checkpoints, not proof that adopted code is now better.

Removing a declaration to erase a hotspot is not a clean refactor. Every removal
should be intentional and documented in the report or review.

### Per-file rollups

`--by-file` gives one row per source file with:

- `functions` and `total_loc`
- `total_effort`
- `max_difficulty`: the largest function-level D in that file
- `max_priority` and the highest-priority `worst` function
- `mean_mi_vs` and `median_mi_vs`, over nontrivial reports

`max_difficulty` is not an invented whole-file Halstead D slapped onto merged
vocabularies. File-level totals aggregate function reports accurately without
pretending that per-function distinct-count sets can be naively combined.

`worst` means highest joint priority, not necessarily greatest D. Use
`--sort difficulty` when specifically finding densest functions.

Removing dense contexts into separate functions can lower maximum D and total
effort while reducing the mean or median MI. That is a **mixed** result and should
be explicit in a review.

### Joint gates and exits

```bash
halmi src/ --min-mi 40 --max-difficulty 30
halmi src/ --max-effort 2.0 --min-band low
```

Halmi reports selected `gates`, `violations`, and `violators`, including the
`name@path:start_line` entries. A violation only exits 1 when **all selected gates
fail**; passing any one enabled floor is enough to clear the invocation.

| Exit | Meaning |
|---|---|
| 0 | Completed analysis with no joint-gate violation |
| 1 | One or more functions fail all selected gates |
| 2 | Usage, configuration, or baseline error |

A permissive band (such as `low`) is not much of a strictness goal and should
not be used to weaken a different threshold accidentally. See crate and CLI
`CONTRACT.md` files for the precise predicate.

### Errors and empty inputs

Malformed inputs become structured errors in `summary.errors`, release builds
keep going with other files, and no-function results use empty arrays instead
of exceptions. If uniform degrade-to-error behavior is not validated for a
specific malformed grammar, that limit belongs in the adapter's no-claim
section rather than being silently accepted as proven.

## Supported languages and contracts

| Adapter | Extensions | Input-model outline |
|---|---|---|
| Rust | `.rs` | `syn`, precise Rust items and macro handling |
| Python | `.py` | Logical and single-line suites, continuations, nesting |
| Go | `.go` | Hand lexer and declarations |
| Swift | `.swift` | Operators/modifiers, raw strings, interpolation |
| Zig | `.zig` | Functions, tests, modifier attributes |
| C | `.c`, `.h` | Function definitions, preprocessor markers, K&R parameters |
| C++ | `.cc`, `.cpp`, `.cxx`, `.hpp`, `.hxx`, `.hh`, `.ipp`, `.tcc`, `.inl` | Scope/member/operator forms, templates and gaps |
| Java | `.java` | Class-body methods, constructors, static initializers |
| C# | `.cs` | Attributes, class/record members, local functions |
| Kotlin | `.kt`, `.kts` | Functions, receivers, secondary constructors, nested comments |
| PHP | `.php` | Named functions and methods, excluding closures/arrow functions |
| Ruby | `.rb` | Def/end methods, modifiers, loop and iterator `do` rules |
| Lua | `.lua` | Named dot and colon functions |
| JavaScript | `.js`, `.jsx`, `.mjs`, `.cjs` | Script adapter: regex, templates, JSX when enabled |
| TypeScript | `.ts`, `.tsx`, `.mts`, `.cts` | Script adapter: same function extraction plus return annotations |

Among all those extensions, each adapter defines its own function inclusion and
exclusion, token type normalization, logical versus physical lines, naming,
class qualification, and no-claim boundaries. Read the concerned crate's
`CONTRACT.md` rather than assuming a total grammar.

## `/halmi`

The bundled workflow retains immutable baselines and produces direct producer
artifacts: reports, metrics, source-spans, and delta counts. Its driver uses
only Python's standard library, so there is no Python package to install.

### Download and invoke

Copy the standalone `skills/halmi` folder and run the Python driver from a
checkout of halmi. Supply `halmi` through `PATH` or `--halmi` when the binary
lives elsewhere.

You can also copy it to a harness-compatible skills directory, for example:

```bash
mkdir -p ~/.agents/skills/halmi/scripts
cp skills/halmi/SKILL.md ~/.agents/skills/halmi/SKILL.md
cp skills/halmi/scripts/cycle.py ~/.agents/skills/halmi/scripts/cycle.py
```

No symlink, private loader directory, particular checkout orientation, or extra
Python dependency is required. An accessible skill definition and its companion
script are the only package contents.

### Agent prompt to copy

```text
/halmi Analyze <TARGET>. Read docs/SCIENCE.md first. Measure once with the
current release binary, then make small evidence-preserving fixes with direct
verification. Preserve the initial immutable baseline.

Inspect Halstead volume, difficulty, effort, MI-VS, MI-original, optional MI-SEI,
CC, LOC, nesting, method-chain data, function names, and spans per function.
Across files, compare max difficulty, total effort, and mean/median MI. Treat
priority as a heuristic, not as scientific proof or an instruction to delete code.

For a semantic bug, reproduce first and retain a failing focused probe before
changing code. For pure refactors, require full-corpus report equality outside
the intentionally changed error or behavior path. Report both improvement and
regression in the least favorable directions. Always use the real program and its
native test lane and `--changed-only` comparison when deltas matter.

Run at most 3 focused repair attempts:
  python3 skills/halmi/scripts/cycle.py <TARGET> --top 5 --output-dir /tmp/halmi-cycle

Run native focused tests after each attempt. Stop on a mixed or unsafe result
instead of weakening tests or changing the original baseline. Cite exact commands,
changed behavior, artifact paths, errors, and everything still unvalidated. Do
not delete unrelated files, change schema definitions, create a branch, commit,
or publish without owner permission.
```

The prompt intentionally asks for evidence and explicit completion criteria.
An empty top-N report is not itself proof of quality. A chosen gate setting,
source semantics, and native tests determine what “green” can legitimately mean.

### Library API

Use adapters with `halmi_core::adapter::LanguageAdapter`, then finalize the
report with the shared priority policy:

```rust
use halmi_core::adapter::LanguageAdapter;
use halmi_core::ranking::{finalize, PriorityConfig};
use halmi_rs::RustAdapter;

let source = "pub fn add(a: u32, b: u32) -> u32 { a + b }
";
let mut reports = RustAdapter.analyze("add.rs", source).functions;
let stats = finalize(&mut reports, &PriorityConfig::default());
assert_eq!(stats.median_effort, 8.0);
```

Only `halmi-rs` uses its Rust-specific parser. The other adapters share
`halmi-lex`'s hand lexer and metric collector. `halmi-lex` is not an existing
adapter name, so do not infer one implicitly. Actual interface crates are listed
in `crates/*/Cargo.toml` and their `CONTRACT.md` files.

per-file `MetricsCollector` retains capacity but restores the original base
nesting depth, which matters for accurate sequential-function analysis.

## Evidence and benchmarks

### Direct validation

Useful focused checks include:

```bash
cargo test -p halmi-core --test formulas
cargo test -p halmi-core --test json
cargo test -p halmi-lex --test lex
cargo test -p halmi-python --test python one_line_suites_and_sequential_defs_keep_their_boundaries
cargo test -p halmi-ruby --test ruby iterator_do_inside_a_loop_remains_a_block
cargo test -p halmi-kotlin --test kotlin nested_block_comments_do_not_expose_declarations
cargo test -p halmi-cli --test cli
cargo doc --no-deps -p halmi-core
cargo audit --no-fetch
```

These cover formulas, schema errors, constructor-base-depth restoration, same-line suites,
Ruby nested `do`, Kotlin comment and expression boundaries, and actual CLI use.
They are evidence for named contracts, not a blanket parser guarantee for every
project's syntax.

The local dependency advisory run uses the 1,352-entry advisory database sample
check available to this project and reports no known findings. That is not proof
of no vulnerabilities.

### Current observed self-run

| Source fixture | Functions | Median effort | MI median | High / Moderate / low |
|---|---:|---:|---:|---|
| halmi source and tests, 63 files | 702 | 1820.612 | 64.117 | 71 / 247 / 358 |

All three full JSON reports were byte-identical with SHA-256:

```text
4af47e3fe930aef22e70c6fc91c4e98f932538f7f053a19972ac8843ca39f129
```

Native-timer observations were 0.02, 0.02, and 0.02 seconds for real elapsed
runs (median 0.02 seconds), 0.12 seconds user CPU, and 0.01 seconds system CPU.
The same source built on Linux (arm64) and Windows (amd64) produced the same
bytes for the JSON, SARIF, table and per-file reports and for the joint-gate
runs.
Source and runtime versions, output digests, and current host load are recorded in
[docs/BENCHMARKS.md](docs/BENCHMARKS.md), so no one has to extrapolate an invented
SLO from these numbers.

## About scientific claims

“Based on the science” does not mean each output field or heuristic restores an
empirical truth. Scientific reasoning requires reproducibility, sensitivity to
context, and named limits of evidence. Here, arithmetic is the beginning of the
trust model, not its conclusion.

Start with:

- [`docs/SCIENCE.md`](docs/SCIENCE.md) for formula provenance and source status.
- [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md) for function-level and external
  observations, their command histories, and their release/access limitations.
- Relevant crate `CONTRACT.md` files for adapter definitions and residuals.

Do not generalize the behavior of one metric or one corpus into a universal
statement of defect likelihood. An aggregate extraction can still preserve
behavior byte-for-byte while leaving mixed Halstead and MI aggregates.

There is currently no validated dataset demonstrating that halmi's own priority
scores, bands, or recommendations predict better employment outcomes. Halmi
should therefore be described as a **deterministic measurement engine** first
and an evidence-guided heuristic only where its documentation explicitly says
so. Scientific claims do not travel farther than their provided data supports.

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md) describes the intended open workflow:
anyone can file an issue, propose a change, fork and make a branch, or open a
pull request. Machine-local harness data, AGENTS.md, and CLAUDE.md stay out of
the distributed project, as does internal or generated state covered by
.gitignore.

Do not treat review integration as permission to fabricate evidence. Bug reports
and PRs should link to reproducible examples, before/after semantics, exact tests,
and residual missing validation. New adapter support needs its `CONTRACT.md`
and semantic regression fixtures.

Releases, tags, packages, and deployments remain maintainer decisions distinct
from public contribution. `LICENSE` is MIT and is retained with the source.

## Related work

[`cccc`](https://github.com/moznion/cccc) is a sister project for separate
cognitive and cyclomatic analyses. Use it where those measurements help, not as
validation of halmi's Halstead or MI accounting, and not as a proxy for a speed
or latency ranking.

The layered API and output points should be:

- `halmi-core`: formula types and report models
- `halmi-lex`: shared token/metric collector behavior
- crate-level `CONTRACT.md`: exact adapter behavior and residuals
- `halmi`: the executable producer of those records

And above all: **prefer a reproducible measurement with bounded authority to an
impressive-sounding claim without a reproducible checking method.**
