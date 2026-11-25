# CONTRACT.md -- halmi-cli

## Purpose and layer

`halmi`, the agent-facing binary: a thin shell over `halmi-core` and the
language adapters. Walks supported source files with a bounded std-thread
pool, ranks by joint priority, emits stable JSON (primary), a human table,
or SARIF 2.1.0, supports baseline/delta and per-file rollups, and turns
joint threshold gates into the exit code. JSON emission and baseline
parsing use halmi-core's dependency-free codec. The CLI itself has no
external runtime dependencies; `serde_json` and `tempfile` are dev-only.
Formulas, ranking, delta, and gate semantics belong to `halmi-core`. Source
classification, spans, and adapter-specific limits belong to each adapter.

## Surface

```
halmi [OPTIONS] [PATHS...]        (defaults to the current directory)

--format <json|table|sarif>   output format (default: json)
--json                        alias for --format json
--top <N>               N highest-priority functions (default 20; N<=0 empty; leading + rejected)
--exclude <GLOB>        skip files whose full path matches a glob (* = any run incl. /, ? = one char); repeatable
--sort <KEY>            order results by priority|effort|difficulty|mi|cc|loc (mi ascending); tiebreak (path, name, start); summary/gates/violators always describe the full set
--min-priority <V>      display floor (finite); applied after gates so exit codes are unaffected
--min-effort/-difficulty/-volume/-loc <V>  Halstead-side display floors, same semantics as --min-priority
--config <FILE>         halmi.toml (auto-loaded from ./halmi.toml): strict zero-dep TOML subset (top/sort/format/color/by_file/min_*/exclude + [gates]); CLI flags beat config values; unknown keys/sections exit 2
--by-file               per-file rollup: JSON gains a top-level files array (path/functions/total_loc/total_effort/max_difficulty/max_priority/mean_mi_vs/median_mi_vs/worst) with empty results; max_difficulty is the maximum function-level Halstead D, not a recomputed whole-file D; MI mean/median cover non-trivial functions; rejected with --baseline or sarif
--color <MODE>          table heat (auto|always|never): red top quartile of displayed rows, yellow next; JSON/SARIF never colored
--baseline <FILE>       compare against a previous halmi JSON run
--changed-only          with --baseline: only worsened functions
--min-mi <V>            gate: maintainability floor (mi_vs)
--max-difficulty <V>    gate: Halstead difficulty ceiling
--max-effort <F>        gate: effort ceiling as multiple of repo median
--min-band <B>          gate: MI band floor (high, moderate, or low)
-h/--help, -V/--version
```

Values may be inline (`--top=5`) or separate (`--top 5`).

Exit codes: `0` success (including "no functions found"); `1` any joint-gate
violation; `2` usage error or unreadable/invalid baseline (message on
stderr).

JSON output: `{ schema_version (2), root, summary, results }`. `summary`
carries `files`, `functions` (pre-`--top` count), `median_effort`, the MI
rollup (`mi_median` plus `mi_bands {high, moderate, low}` over non-trivial
functions), `languages` (function counts per language), `errors[]` (skipped
when empty), and optional `delta` / `gates` blocks. Numbers use shortest round-trip formatting; every report float
carries at most 3 decimals.
`results` are ranked `FunctionReport`s (see the halmi-core contract for the
field set; `delta`/`delta_status` appear only in baseline mode). The
baseline file may be a previous full run (`{"results": [...]}`) or a bare
array of results.

`--format sarif` emits SARIF 2.1.0 (schema
`https://json.schemastore.org/sarif-2.1.0.json`), one run, tool driver
`halmi` + version. Every non-trivial function in the emitted top-N becomes a
result: rule id `HALMI001` ("Joint Halstead + Maintainability Index
hotspot"), `level` = `warning` when the function violates the joint gates
else `note`, message = name + reason + key numbers, location = the function
span (artifact uri = report path, startLine/endLine). Trivial functions are
excluded. Exit-code semantics are identical to JSON mode.

## Invariants

1. Gates are evaluated over every analyzed function BEFORE `--top`
   slicing (`tests/cli.rs::gates_set_exit_code`); `summary.gates.violators`
   (`name@path:start_line`) is computed over the full set and never hidden
   by slicing (`tests/cli.rs::gates_name_their_violators`). Gate values
   must be finite (`tests/cli.rs::nonfinite_gate_values_are_rejected`);
   the stderr summary uses each label exactly once
   (`tests/cli.rs::gate_stderr_has_exact_single_labels`). Directory
   symlinks are never followed
   (`tests/cli.rs::symlinked_directories_are_skipped`). Baseline runs warn
   on stderr when the large majority of functions are new (truncated or
   foreign baseline) (`tests/cli.rs::baseline_mismatch_warns_on_stderr`).
   A directory as `--baseline` is a clear usage error; `--` ends flag
   parsing; `--exclude` filters the final list by full-path glob
   (`tests/cli.rs::exclude_patterns_filter_files`).
2. Parse/IO failures never fail the run: the file is skipped and recorded
   in `summary.errors`; other files keep going
   (`tests/cli.rs::parse_errors_do_not_crash`).
3. A violation message on stderr names every enabled gate, disabled ones as
   `-` (`tests/cli.rs::gates_stderr_names_every_enabled_gate`).
4. Directory walk skips `target/`, `node_modules/`, `venv/`, `.venv/`,
   `site-packages/`, and dot-directories; collects `.rs`, `.py`, `.go`, and
   `.js/.jsx/.ts/.tsx/.mjs/.cjs/.mts/.cts` files; deduplicates and sorts. Each file is routed to its adapter by
   extension (Rust default). Non-source path arguments are skipped with a
   stderr note. Passing a single source file analyzes it with the path
   verbatim (`tests/cli.rs::single_file_argument_is_analyzed_verbatim`).
5. `--changed-only` without `--baseline` is a usage error (exit 2)
   (`tests/cli.rs::changed_only_without_baseline_is_rejected`).
6. Default with no PATHS is the current directory, reported as root "."
   (`tests/cli.rs::defaults_scan_the_current_directory`).
7. Trivial functions surface with `priority 0` and `mi_vs 100` and stay in
   the output (agents see the full inventory) (`tests/cli.rs::
   json_output_shape_and_ranking`).
8. SARIF results keep the ranked order of the emitted slice; a result's
   `level` is `warning` exactly when that function is in the gate-violation
   set computed over the full ranked list; trivial functions never appear
   (`tests/cli.rs::sarif_output_shape`,
   `tests/cli.rs::sarif_marks_gate_violations_as_warning`).

## Error model

Two channels: structured degradation (`summary.errors` entries for
per-file problems) and process-level errors (exit 2 + `halmi: <message>` on
stderr for usage/argument/baseline problems). Gate violations are NOT
errors: exit 1 with the JSON still printed on stdout.

## Determinism class

Deterministic: sorted file enumeration, per-file analysis in parallel but
results merged in file order, ranking from `halmi-core` is deterministic.
Identical trees produce identical JSON on every platform: report paths are
rendered with `/` separators and files sort by that rendering, so a baseline
captured on Windows matches on macOS or Linux. The table format truncates
names at 28 columns on char boundaries.

## Cancellation behavior

No cancellation protocol: a SIGTERM kills the process (worker threads
terminate with it); output is emitted in one `println` at the end, so a
killed run produces no partial JSON. Cancellation is clean by omission, not
by checkpointing.

## Unsafe boundary

None. `#![deny(unsafe_code)]` is set at the crate root.

## Feature flags

None.

## Conformance tests

`tests/cli.rs` -- 38 end-to-end tests driving the real binary on fixture
trees: JSON shape/ranking + MI rollup + 3-decimal wire floats, table
output, SARIF shape + warning levels, gate exit codes and stderr message,
baseline/changed-only delta mode, parse-error degradation, defaults, flag
forms, single-file input, unicode-safe table truncation. The SARIF tests
were written first and failed against the no-SARIF build (`unknown format
sarif`, exit 2) before implementation. serde_json is a dev-dependency used
only to independently validate the hand-rolled output in tests; the thin
CLI shell has zero external dependencies, while `halmi-rs` intentionally
uses its parser library.

Mutation-verified: changing the default scan path away from "." fails the
defaults test; the gate-message test was written first and failed against
the original message (which omitted the effort factor) before the fix.

## No-claim boundaries

- The table format is human-facing and not a stability contract; the JSON
  schema (`SCHEMA_VERSION 2`) is. The SARIF mapping (one rule id, level
  semantics, trivial exclusion) is documented here and covered by tests, but
  is halmi's own mapping, not a standardized SARIF profile.
- Benchmark provenance is in `docs/BENCHMARKS.md`: exact commands, fixtures,
  host load, and reproducibility limits belong with every timing claim; there
  is no universal latency SLO.
- `--max-effort` compares against the median of the current run only.
