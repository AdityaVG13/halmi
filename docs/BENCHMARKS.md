# halmi benchmarks and evidence

This file distinguishes reproducible measurement from observations on external
sources. Numbers do not certify overall quality, defect prediction, or universal
maintainability. Raw time and delta metrics are not scientific proof of how
long a human will take to review or repair code.

Read [SCIENCE.md](SCIENCE.md) for formula provenance and interpretation. See
[CONTRIBUTING.md](../CONTRIBUTING.md) for what to record before publishing a new
benchmark. The public language and schema contracts explain the values being
reported.

## Reproducible halmi self-run

### Measurement basis

- Fixture: the current source workspace, including tests and Python tooling.
- Binary: `target/release/halmi`, built with Rust 1.100.0.
- Command:
  `/usr/bin/time -p target/release/halmi . --json --top 100000`
- Host: ARM64 Mac, macOS 26.6.2.
- System load average sampled right after the three runs: 3.51 (1 minute),
  3.88 (5 minutes), 3.91 (15 minutes). These were not idle-machine benchmark
  conditions.

Executed this command three times using the same source tree. These are full
report timings from `/usr/bin/time`, not raw evaluation-only timings, so they
include JSON output, traversal, parsing, operation counting, and the host's
wall-clock precision.

| Invocation | real (s) | user (s) | sys (s) | Median summary effort | MI median |
|---|---:|---:|---:|---:|---:|
| 1 | 0.02 | 0.12 | 0.01 | 1820.612 | 64.117 |
| 2 | 0.02 | 0.12 | 0.01 | 1820.612 | 64.117 |
| 3 | 0.02 | 0.12 | 0.01 | 1820.612 | 64.117 |
| median | 0.02 | 0.12 | 0.01 | 1820.612 | 64.117 |

The median wall time was 0.02 s and median user CPU time was 0.12 s. Do not
convert this into an exact rate or extrapolate to other hosts. `/usr/bin/time`
uses limited precision for this output, and the runs included unrelated host
load. A dramatic cold/warm wall-time difference is an observation, not a launch
speedup claim.

### Corpus and output

- **63 source files**
- **702 functions**
- **1820.612 median Halstead effort**
- **64.117 median MI-VS**
- **71 high, 247 moderate, 358 low MI-band functions**

The function total includes nested and test functions included by the current
Rust adapter, not only `main` implementations. Other fields must not be inferred
from those counts. The most prominent observed files reflect lexical scanners
more than scientific literature, so their workload is appropriate for dogfooding
but not for a general benchmark of all codebases.

### Artifact digests

All three full JSON reports are byte-identical. Their shared SHA-256 is:

```text
4af47e3fe930aef22e70c6fc91c4e98f932538f7f053a19972ac8843ca39f129
```

The byte identity applies to fixed analyzed files and invocation, not to quality.
It demonstrates deterministic output for identical source and runtime inputs,
not reproducibility of latency on a busy shared host. The native cross-platform
check below covers the other output formats and exit codes.

- `/tmp/halmi-benchmark-readme-1-aa6faaf576b044a3aa6bf5f1475b5f61.json` (timer output in the
  sibling `.time` file)
- `/tmp/halmi-benchmark-readme-2-0d3652dba04e401a97ba797dd76776c2.json`
- `/tmp/halmi-benchmark-readme-3-f44f5b0665bb4177aea5906bd50ffc6c.json`

Rebuild and re-run the same fixture to see the report rather than relying on
interpolated sample text. Save new outputs to `/tmp/halmi-benchmark-readme-*.json`.

```bash
rustup run 1.100.0 cargo build --release -p halmi-cli
/usr/bin/time -p target/release/halmi . --json --top 100000 > /tmp/halmi-benchmark-readme-your-run.json
```

### Native cross-platform equality

The 110 public files of this tree were archived, extracted on three hosts
and built with `cargo build --locked --release -p
halmi-cli --bin halmi` under the pinned Rust 1.100.0 toolchain: macOS 26.6.2
(arm64), Linux (arm64) and Windows (amd64, MSVC). Each binary then analyzed
its own extracted tree. Every stdout byte and every exit code matched:

| Invocation | Bytes | SHA-256 prefix | Exit |
|---|---:|---|---:|
| `halmi . --json --top 100000` | 301895 | `4af47e3fe930aef2` | 0 |
| `halmi . --format sarif` | 10756 | `303d0c718e0296e9` | 0 |
| `halmi . --format table` | 2465 | `8eb552727ee72f57` | 0 |
| `halmi . --by-file` | 5414 | `dbf55c12d445acf7` | 0 |
| `halmi . --min-mi 50` (134 violations) | 17472 | `b5d0aa5a701d8978` | 1 |
| `halmi . --min-mi 20` (2 violations) | 10757 | `a1eb9c2d7a6104ee` | 1 |
| `halmi . /nonexistent --json --top 100000` | 301909 | `2a744d41c57bbace` | 0 |
| `halmi . --top abc` | 0 | empty | 2 |

The release binaries were 2841808 bytes (macOS), 3133208 bytes (Linux) and
3194880 bytes (Windows). The archived source (110 files, `tar` with
`COPYFILE_DISABLE=1`, no xattrs) had SHA-256
`669307dcb625dac812acc6f78ff4d8316347e8e513812c3333672c5920562df7`. This is fixed-input equality on one tree, not a
performance claim and not proof over other corpora.

## Before/after behavior comparisons

For pure refactors, require full corpus JSON identity rather than showing a small
subset and calling it parity. This also guards against changing top-N selection
or comparing the wrong file set.

Session evidence for the 0.1.0 candidate (2026-11-24; local corpora, not
vendored, so historical rather than reproducible from this repo): the candidate
and a pre-session binary ran 96 fixed CLI cases over nine local trees (MRX,
TensorAST, ast-sgrep, zero, pi-stack, emath, fast-context, a Rust snapshot and a
fixture). 87 cases were byte-identical. The 8 TensorAST cases differ because the
pre-session binary panicked (exit 101) on a Go grouped function type and the
candidate completes. The 8 MRX cases differ in one Go file of 22,411, an
`interface{}` return type the pre-session scanner cut short; the Rust, Python,
Zig, C, C++, Swift, JS/TS and Kotlin rollups were identical over 333,563
functions. The intentional parser fixes (Go body-less declarations, Ruby endless
methods and setters, C# attributes) are outside the identical set by design.

For intentional behavior fixes and source refactors, compare:

- Operator and operand counts and the derived Halstead suite
- MI-VS, MI-original, and MI-SEI where present
- CC, LOC, nesting, and method-chain signals
- Start and end lines, names, list membership, and errors
- Per-file maximum difficulty and total effort
- Per-file mean and median MI, not only the former worst function

The output must state whether all these measurements improved, moved in mixed
directions, or merely relocated dense logic to new helpers. Never report a focus
change as uniformly positive by omitting a metric regression.

If using a different target's baseline, preserve it. `--changed-only` can remove
matching unchanged rows from visibility, but it does not prove that the baseline
is canonical or that newer source is a better baseline. Deleting a function to
silence a metric is an explicit semantic change, not a clean refactor.

## Different kinds of evidence

1. **Formula conformance.** Exact hand-computed identities and reference values
   such as the `a + b` example in [SCIENCE.md](SCIENCE.md).
2. **Adapter conformance.** Discriminating source syntax unit/integration tests.
3. **Parser-span differentials.** Function starts/ends against supported
   tree-sitter dump2 outputs (a private oracle tool, not in this repository;
   see SCIENCE.md section 8). This doesn't validate classifications separately.
4. **Problem-focused regressions.** EOF and Unicode errors, nested comments and
   blocks, negative control cases, boundary changes, and state resets.
5. **Performance and scale observations.** Actual command timing and host load,
   with output hashes and fixtures described exactly.
6. **Outcome studies.** Empirical work on whether these recommendations save
   review time or reduce defects. This does not yet exist for halmi's own heuristic
   priorities, and borrowing results from different studies cannot supply it.
