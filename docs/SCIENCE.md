# Scientific basis, interpretation, and validation

This document separates what halmi computes from what halmi claims. It is the
public provenance guide for operators, operands, Halstead metrics,
Maintainability Index (MI), rankings, and per-file rollups.

The short version:

- The core Halstead and MI-VS arithmetic is explicit and testable.
- Operator/operand classification is a measurement convention that must be
  checked independently for each language adapter.
- Priority, thresholds, reasons, and delta classifications are heuristics. They
  are not fitted probabilities, universal quality scores, or causal findings.
- Better metric output is not, by itself, evidence that software has become
  cheaper to maintain or less defective.

## 1. Claims by evidence level

| Level | Meaning | halmi examples |
|---|---|---|
| Formula | An explicit equation can be recomputed from raw inputs | `V`, `D`, `E`, original MI, MI-VS |
| Measurement convention | A documented choice determines the raw inputs | operators vs operands, function spans, logical or physical LOC, one declaration operator |
| Implementation evidence | Tests constrain a specific implementation behavior | hand-computed fixtures, identity grids, exact corpus report equality |
| Heuristic | An opinionated interpretation or threshold guides action | priority coefficients, `reason`, 65/85 MI bands, gate/delta thresholds |
| External validation | Evidence on other codebases links a metric to an outcome | review effort, change failure, defects, repair time |

Halmi currently has substantial evidence at the first three levels and heuristic
rules at the fourth. It has not established the fifth for its own priority,
bands, gates, or recommendations. Published studies about other datasets and
implementations cannot be promoted into that evidence.

## 2. Halstead quantities used by halmi

For each function, adapters collect:

- `eta1`: distinct operator kinds.
- `eta2`: distinct operand spellings or values, under the adapter's rule.
- `N1`: total operator occurrences.
- `N2`: total operand occurrences.

The central computed fields are:

```text
eta = eta1 + eta2
N   = N1 + N2
V   = N * log2(eta)
D   = (eta1 / 2) * (N2 / eta2)
E   = D * V
```

`volume` is V, `difficulty` is D, and `effort` is E. Degenerate counts are marked
non-meaningful instead of producing NaN or infinity. `FunctionReport` exposes
these derived values; `RawFunction.counts` and `HalsteadCounts` expose the raw
counts to adapter and library consumers.

Other Halstead fields are secondary. In particular:

- `estimated_bugs = V / 3000` is a relative diagnostic, not a predicted bug
  count.
- `time_seconds = E / 18` uses Halstead's Stroud convention and has no halmi
  uncertainty estimate.
- `purity_ratio = estimated_length / N` is an implementation-shape signal, not a
  correctness measure.
- `potential_volume` depends on approximated unique input/output parameters.
- Alternate estimated-length fields are experimental and are not used by the
  priority formula.

NIST Technical Note 1990 is directly relevant to these limits. It says
reproducible operator/operand counting needs a measurement standard, questions
the interpretation of code-attribute counts as unobservable mental events, and
identifies the inputs to potential volume as questionable. Halmi therefore does
not claim an objective cognitive-effort measurement.

## 3. Maintainability Index forms

### Original unbounded form

```text
MI_original = 171
            - 5.2  * ln(V)
            - 0.23 * CC
            - 16.2 * ln(LOC)
```

### Visual Studio normalization

```text
MI_VS = max(0, MI_original * 100 / 171)
```

This is the formula published directly by Microsoft. Microsoft's green, yellow,
and red thresholds are 20-100, 10-19, and 0-9. Halmi's `high >= 85`,
`moderate >= 65`, and `low < 65` bands are a different, project-selected scheme.
One scheme must never be attributed to the other.

### SEI comment variant

Halmi additionally offers:

```text
MI_SEI = MI_original + 50 * sin(sqrt(2.4 * clamp(CM, 0, 1)))
```

`CM` is comment-line density as reported by the adapter. This value is not
MI-VS and is not normalized to the same range. The internal formula document
records this historical variant, but a direct primary SEI text for the term was
not independently established in the current source review. Treat its
provenance as unfinished rather than merely asserting "standard".

`mi_vs`, `mi_band`, and priority do not depend on `mi_sei`, estimated bug count,
estimated time, potential volume, or alternative estimated lengths.

### Very small functions

Halmi defines non-meaningful Halstead inputs or zero LOC as original MI and
MI-VS 100, and excludes those functions from priority and MI rollups. When
comment density is available, MI-SEI is also 100; when it is absent, MI-SEI is
absent rather than being synthesized as 100. This is a halmi convention, not a
Microsoft rule or a measurement of exceptional quality.

## 4. Halmi's measurement conventions

The following choices affect numeric results and are not universal laws:

| Choice | Status and consequence |
|---|---|
| What is an operator or operand | Adapter-specific measurement definition. The authoritative rules are the adapter's `CONTRACT.md` and tests. |
| Unit of analysis | Metrics are per function. A whole file is not fed through the formula as though it were one function. |
| Declaration accounting | Exactly one declaration operator is added per function by the shared collector. |
| Function signatures | Signatures shape the span and parameter inputs but different adapters include different token sets in body counts. |
| LOC | Adapters document whether they use logical or physical code lines. These values are not identical to MSIL-based Visual Studio LOC. |
| Nested definitions and macros | Inclusion or exclusion is adapter-specific and documented in its contract. |
| Per-file difficulty | `max_difficulty` is the maximum function D. It is not recomputed from a fake file-wide union of distinct counts. |
| Per-file effort | `total_effort` is the sum of function efforts. It is an aggregate over reports, not a new independent psychological estimate. |
| Per-file MI | `mean_mi_vs` and `median_mi_vs` cover non-trivial functions. Adding or splitting functions changes this population and can move both statistics. |

Because classification defines all downstream counts, tree-sitter function-span
parity is not an independent operator/operand oracle. Hand-counted fixtures and
independent classification checks remain necessary. Cross-language D, E, MI, or
priority comparisons are unsafe unless the counting conventions are shown to be
commensurate.

## 5. Opinionated product rules, explicitly named

None of these values has been calibrated against an external outcome dataset:

- Priority uses repository-relative effort, `100 - mi_vs`, nesting, and LOC.
- `PriorityConfig` defaults: `nest_weight = 0.15`, `nest_free = 2`,
  `size_weight = 0.2`, and `effort_floor = 1`.
- `PRIORITY_EPSILON = 0.5` and `MI_DROP_THRESHOLD = 50` affect delta labels.
- Reason strings and their thresholds suggest what to inspect; they do not
  identify a defect or prescribe a proven repair.
- Enabled gates use joint fail-all semantics. This is a product policy, not a
  formula from the MI or Halstead literature.
- `cccc` is a sister tool for independent cognitive and cyclomatic analyses. A
  `cccc` rank or score is not evidence of Halstead or MI validity and does not
  calibrate halmi.

Refactoring can lower the former hotspot's D and priority while lowering a
file's mean or median MI. Such a result is mixed, not "improved" without
qualification. Changing the number of functions also changes MI mean, median,
and band-count interpretation.

## 6. Sources and what they support

The source label matters. Bibliographic verification of a publication is not
presented as a claim that its full text was inspected.

| Source | What it is used for here | Inspection status |
|---|---|---|
| M. H. Halstead, *Elements of Software Science* (1977), ISBN `0-444-00205-7` | Primary historical basis for the Halstead family | Bibliographic record and restricted-scan metadata inspected; full text was not available |
| [Verifysoft, Halstead Metrics](https://www.verifysoft.com/en_halstead_metrics.html) | Secondary restatement of `D`, `E`, `T`, and related forms | Full web text inspected |
| P. Oman and J. Hagemeister, "Metrics for Assessing a Software System's Maintainability" (1992), DOI `10.1109/ICSM.1992.242525` | Original MI lineage | Crossref/OpenAlex metadata inspected; full paper was closed |
| D. Coleman, D. Ash, B. Lowther, and P. Oman, "Using Metrics to Evaluate Software System Maintainability" (1994), DOI `10.1109/2.303623` | MI model and evaluation lineage | Crossref/OpenAlex metadata inspected; full paper was closed |
| [Microsoft, Maintainability index range and meaning](https://learn.microsoft.com/en-us/visualstudio/code-quality/code-metrics-maintainability-index-range-and-meaning) | Exact original and normalized MI-VS equations plus Microsoft's thresholds | Full current documentation inspected |
| [David Flater, NIST TN 1990, *'Software Science' revisited* (2018)](https://doi.org/10.6028/NIST.TN.1990) | Measurement-standard, dimensional, effort-model, and potential-volume cautions | Full public technical note inspected |
| ["Exploring Maintainability Index Variants..." (2023), DOI `10.3390/app13052972`](https://doi.org/10.3390/app13052972) | Variant choice changes perception, especially at fine granularity; long-term trends correlate | Abstract inspected through OpenAlex; full article was not available to this review |
| [Khan and Nadeem, "Evaluating the effectiveness of decomposed Halstead Metrics in software fault prediction" (2023)](https://doi.org/10.7717/peerj-cs.1647) | Fault-prediction evidence is dataset-, model-, language-, and validity-sensitive; base/decomposed metrics differ from derived metrics | Full article sections, results, threats to validity, and conclusion inspected |

These references establish the formulas' lineage and the limits of interpreting
such studies. They do not validate halmi's adapters, priority weights, bands,
gates, or output on a user's repository.

## 7. A hand-checkable example

Source:

```rust
pub fn add(a: u32, b: u32) -> u32 {
    a + b
}
```

Under the documented Rust adapter convention:

- Operators: `fn` and `+`, so `eta1 = 2`, `N1 = 2`.
- Operands: `a` and `b`, so `eta2 = 2`, `N2 = 2`.

Therefore:

```text
eta = 4
N   = 4
V   = 4 * log2(4) = 8
D   = (2 / 2) * (2 / 2) = 1
E   = 1 * 8 = 8
CC  = 1
LOC = 1
```

And independently of the implementation:

```text
MI_original = 171 - 5.2 * ln(8) - 0.23 - 16.2 * ln(1)
            = 159.956903983...
MI_VS       = 93.542 after report rounding
```

The current implementation reports `volume = 8`, `difficulty = 1`,
`effort = 8`, `cyclomatic = 1`, `loc = 1`, and `mi_vs = 93.542`. The executable
oracle for the derived behavior is
`crates/halmi-rs/tests/analysis.rs::simple_function_counts_exactly`.

Run the formula and adapter checks through the repository's own environment:

```bash
cargo test -p halmi-core --test formulas
cargo test -p halmi-rs --test analysis simple_function_counts_exactly
```

The adapter's exact classification of `fn add` is an additional convention; the
algebra alone does not prove that convention.

## 8. Evidence the repository currently provides

| Check | What it proves | What it does not prove |
|---|---|---|
| Formula reference values and count-grid identities | The core arithmetic and degenerate behavior match the written equations | That an adapter classifies real code correctly |
| Adapter fixtures | Named examples have intended spans, classifications, and derived values | Universal coverage of every syntax form |
| Tree-sitter differential spans (maintainer-local; not runnable from this repository) | Function inventory and spans matched an independent parser within the documented residual set, on the corpora named in each contract | Operator/operand correctness or MI/Halstead validity |
| Real-corpus runs | Files are processed without crashes and known residuals remain explicit | Generalization to every codebase or language |
| Before/after corpus report hashes | A refactor leaves measured behavior byte-identical on that fixed corpus | Improved maintainability, or absence of behavior outside the corpus |
| Halmi's self-rankings | Which functions this implementation selects under current conventions and heuristics | That the ranking predicts user outcomes better than another ranking |

Each adapter contract is the accounting area for its corpus, residuals, and
no-claim boundaries. Historical summaries must be read with the command and
corpus named there; they are not canonical measurements on unrelated trees.

The tree-sitter oracle is a private developer tool. It is not in this
repository, nothing here builds it, and the corpora it ran on are local to the
maintainer's machines. Every tree-sitter figure quoted in a crate
`CONTRACT.md` (the "Differential vs tree-sitter-X" paragraphs) is therefore a
historical local observation: UNREPRODUCIBLE from this repo, with the missing
harness (the oracle binary) and the unvendored corpus named alongside it.
What a clone can reproduce is the per-crate suites and the CLI E2E tests.

`tools/oracle_diff.py` is shipped only so that anyone holding an equivalent
dump tool can rerun the comparison:

```bash
python3 tools/oracle_diff.py target/release/halmi <mode> <limit>
```

Optional corpora are never vendored. Each mode reads its corpus from
`HALMI_CORPUS_<MODE>` (for example `HALMI_CORPUS_SWIFT`): glob patterns
separated by the platform `PATH` separator, where a plain directory is walked
recursively for the mode's extensions. `HALMI_CORPUS_EXCLUDE` lists path
substrings to skip (default: build and dependency directories). `HALMI_ORACLE`
names the dump tool; if it is unset, `ta-oracle` is looked up on `PATH`, a
name only the maintainer's machine resolves. An unset corpus, a missing
oracle, or an empty selection is a configuration error (exit 2), not a
successful comparison.

The interface the harness consumes from that tool: it is invoked as
`<tool> <lang> --dump2` with one source file on stdin and the mode's language
id (`swift`, `c`, `cpp`, `java`, `c_sharp`, `kotlin`, `go`, `php`, `ruby`,
`lua`, `zig`; `script` mode passes `tsx`, `typescript` or `javascript` by file
extension). It prints one tab-separated row per syntax node with at least
eight columns: the node kind in column 2, the 0-based start line in column 5
and the 0-based end line in column 7. Rows whose kind is `ERROR` carry byte
offsets in columns 3 and 4; a file whose `ERROR` rows cover more than 30% of
its bytes is reported as an oracle parse failure, not as a span mismatch. A
non-zero exit marks the file as skipped and reported.

The harness records language, corpus roots, evaluated files, span matching,
and mismatches. Oracle and adapter failures are reported, non-files are excluded
explicitly, and an empty selection or zero usable comparisons is an error rather
than a clean result. For modes whose adapters count definitions only (Go),
oracle declarations whose span opens no block are body-less signatures and
are excluded from both sides, with the count printed. It does not compare a
second independent Halstead implementation.

## 9. A validation protocol worth trusting

For implementation-preserving refactors:

1. Fix source, config, corpus, and binary versions.
2. Run focused formula and adapter tests.
3. Compare full corpus reports, not only top-N output, and retain mismatches.
4. Re-run both halmi and `cccc` when their separate signals are relevant.
5. Record max D, total E, minimum/mean/median MI-VS, function count, and LOC.
   State mixed MI distributions explicitly.

For claims that halmi helps users:

1. Define an outcome before inspecting results: review time, accepted
   recommendations, change failure, post-release defects, or repair time.
2. Keep corpus, labeling, and project/time holdouts separate from every tuning
   decision.
3. Compare against simple LOC/CC rankings and existing review practice, not only
   against halmi with changed weights.
4. Use suitable ranking/calibration measures and uncertainty estimates. Raw
   accuracy and a convenient threshold are insufficient for imbalanced defects.
5. Publish misses, dataset shift, language limits, and the exact command/corpus
   for every reproduced result.

A threshold or priority weight should move from "halmi heuristic" to "calibrated
on a named corpus" only after that process. A universal maintainability or
fault-probability claim requires external evidence across projects and time.

## 10. Current honest conclusion

Halmi is a deterministic way to compute and compare a documented family of
source metrics. Its core D, V, E, original MI, and MI-VS arithmetic is inspectable
and tested. Adapter classification and corpus residuals remain part of the
result, and the ranking layer is intentionally opinionated but uncalibrated.

The defensible use today is evidence gathering and within-context comparison.
It is not "halmi has scientifically proven this function is bad, this threshold
is universal, or this refactor improved maintainability for every project".
