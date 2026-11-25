# CONTRACT.md -- halmi-core

## Purpose and layer

Pure metric engine: Halstead software-science suite, Maintainability Index
variants + banding, the unified `FunctionReport`, joint priority ranking,
baseline/delta comparison, threshold gates, the language-neutral
[`adapter`] port, and a dependency-free [`json`] codec for halmi's own
schema. Bottom layer of the workspace: no IO, no language-specific parsing,
no async, **no dependencies**. Everything here is a total, synchronous,
deterministic function over plain data. Upstream consumers: `halmi-rs`
(adapter), `halmi-cli` (binary), and any external library user.

Formula source of truth: `docs/internal/halmi-formulas.md` and
`docs/internal/halmi-integration.md` (local, gitignored). Public provenance,
interpretation limits, and validation protocol: `docs/SCIENCE.md`.

## Public types and semantics

All items are re-exported from the crate root or reachable as
`halmi_core::<module>::<item>` (all five modules are `pub`).

`metrics`:

- `HalsteadCounts { eta1, eta2, n1, n2 }` -- raw operator/operand counts
  (distinct kinds and total occurrences). `Copy`, `Default`.
- `HalsteadCounts::compute() -> HalsteadMetrics` -- the full suite:
  `vocabulary`, `length`, `volume`, `difficulty`, `effort`, `level`,
  `time_seconds`, `estimated_bugs`, `estimated_length`, `purity_ratio`,
  `intelligence`, `language_level`, `alt_length_factorial`,
  `alt_length_cross`, `alt_length_root`, `potential_volume`, `meaningful`.
- `MiInputs { volume, cyclomatic, loc, comment_density: Option<f64> }`,
  `MiMetrics { original, vs, sei: Option<f64>, meaningful }`,
  `compute_mi(&MiInputs, halstead_meaningful) -> MiMetrics`.
- `HalsteadCounts.io_params` (eta2*: unique input/output parameters) and
  `HalsteadMetrics.potential_volume` (V* = (2 + eta2*) * log2(2 + eta2*));
  0.0 when the suite is not meaningful. The Rust adapter derives eta2* from
  signatures (distinct parameter names, `self` included, + 1 for a non-unit
  return); adapters without signature access report 0 (V* = 2).
- Constants `STROUD_NUMBER = 18.0`, `BUGS_VOLUME_DIVISOR = 3000.0`.

`report`:

- `SCHEMA_VERSION: u32 = 2` -- version of the JSON wire format (v2 adds the
  additive `mi_band`; v1 baselines still parse, band recomputed from mi_vs).
- `Language { Rust, Python, Go, Swift, Zig, C, Cpp, Java, Cs, Kotlin, Php, Ruby, Lua, JavaScript, TypeScript, Other }` -- per-report source language
  (`as_str`/`parse`); absent in old baselines, defaults to `Other`.
- `MiBand { Low, Moderate, High }` (derived Ord: Low < Moderate < High) +
  `MiBand::of(mi_vs)`: the documented 85/65 banding of `mi_vs`.
- `FunctionReport.method_chain` + `RawFunction.method_chain`: longest
  method-call spine (idiom signal; adapters without chains report 0;
  optional on parse, defaults 0).
- `DeltaStatus { New, Worsened, Improved, Unchanged }` (replaces the former
  stringly status; `New` never carries a `DeltaInfo`).
- `FunctionReport::to_json_string`, `FunctionReport::from_json(&Json)`,
  `write_report_json` (the single owner of the field list/order/optionality),
  `parse_reports` (accepts `{"results": [...]}` or a bare array).
- `RawFunction` -- everything an adapter collects in one AST walk; converted
  with `FunctionReport::from_raw(&RawFunction)`.
- `FunctionReport` -- the stable JSON unit agents consume. Floats are rounded
  to 3 decimals at construction, and `finalize` rounds the `priority` it
  sets to the same wire precision. `delta` / `delta_status` are
  `#[serde(skip_serializing_if = "Option::is_none")]`.
- `DeltaInfo { priority, mi_vs, effort }` -- current minus baseline.

`ranking`:

- `PriorityConfig { nest_weight, nest_free, size_weight, effort_floor }`
  (defaults 0.15 / 2.0 / 0.2 / 1.0) -- tunable, not empirically calibrated.
- `compute_priority(cfg, effort, mi_vs, nesting, loc, median_effort) ->
  f64`: `(effort / max(median, effort_floor)) * (100 - mi_vs).max(0) *
  (1 + nest_weight * max(0, nesting - nest_free)) * (1 + size_weight *
  max(0, ln(loc)))`.
- `finalize(&mut Vec<FunctionReport>, &PriorityConfig) -> RepoStats` --
  fills `priority` + `reason`, sorts in place (see invariants), returns
  `{ median_effort, meaningful }`.
- `is_trivial(&FunctionReport) -> bool`, `reason_for(...) -> String`
  (at most two `"; "`-joined clauses).

`delta`:

- `attach_delta(&mut [FunctionReport], &[FunctionReport]) -> DeltaStats` --
  matches on (name, path) with one-to-one pairing per name; attaches
  `delta` + `delta_status` in {`worsened`, `improved`, `unchanged`, `new`}.
- `paths_match(a, b)` -- exact, or either is a path suffix of the other
  after `\` -> `/` normalization (baseline recorded under a different root).
- `retain_worsened(&mut Vec<FunctionReport>)` for `--changed-only`.
- Constants `PRIORITY_EPSILON = 0.5`, `MI_DROP_THRESHOLD = 50.0`.

`adapter` (the isomorphic language port):

- `SourceAnalysis { path, functions, error }` -- language-neutral per-file
  result; defined here, produced by adapters.
- `LanguageAdapter { language(), analyze(&self, path, src) -> SourceAnalysis }`
  -- pure (no IO), never panics, one pass per function for all raw signals.
  A new language is a new crate implementing this trait; everything
  downstream is language-neutral by construction.

`json`:

- Writer helpers `write_string` (full escaping incl. control chars) and
  `write_f64` (finite; shortest round-trip formatting), a `Json` value type
  with order-preserving objects, and `parse` (recursive descent, surrogate
  pairs, rejects trailing bytes and raw control chars). Validated against an
  independent parser in tests (serde_json, dev-dependency only).

`gates`:

- `Gates { min_mi, max_difficulty, max_effort_factor: Option<f64>,
  min_band: Option<MiBand> }`,
  `violates_all(&Gates, &FunctionReport, median_effort) -> bool`
  (min_band fails when the band is strictly lower than required),
  `violations(...) -> Vec<usize>` (report order).

## Invariants

Each invariant is pinned by the named conformance test.

1. Meaningfulness rule: `meaningful == (n1 > 0 && n2 > 0 && eta1 + eta2 > 1)`;
   when false, every derived Halstead field is exactly `0.0` (no NaN, no
   division by zero, no log of <= 1). -- `tests/formulas.rs::
   halstead_degenerate_counts_are_not_meaningful`
2. Cross-field identities hold for every meaningful count vector:
   `E = D*V`, `T = E/18`, `B = V/3000`, `L*D = 1`, `I = L*V`,
   `lambda = L^2*V`. -- `tests/formulas.rs::
   halstead_suite_identities_hold_across_count_grid`
3. Known hand-computed values for the reference counts {4,3,7,6} and the MI
   variants (original/vs/sei) are exact to 1e-9. -- `tests/formulas.rs`
4. Tiny functions (non-meaningful Halstead or `loc == 0`) have `meaningful ==
   false`, original MI = 100 and MI-VS = 100; `mi_sei` is 100 when comment
   density is provided and `None` otherwise. `vs` never drops below 0; SEI
   comment density is clamped to 0..=1. -- `tests/formulas.rs` and
   `tests/report.rs::from_raw_trivial_maps_to_maintainability_100`
5. `finalize` sort order: priority desc, then effort desc, then
   (path, name, start_line) asc -- total, deterministic, idempotent;
   trivial functions get `priority == 0.0` and reason "trivial function"
   and are excluded from the median. -- `tests/ranking.rs`
6. Gate semantics: a violation requires failing *every* enabled gate (and at
   least one enabled); trivial functions never violate. --
   `tests/gates.rs`
7. Delta statuses: `worsened` when `delta.priority > 0.5` OR
   (`mi_vs < 50` && mi dropped && effort rose); `improved` when
   `delta.priority < -0.5`; else `unchanged`; unmatched -> `new` with no
   `delta` payload. -- `tests/delta.rs`
8. JSON: every documented field is always serialized (except `mi_sei`
   opt-out shape and the delta pair when absent); round-trips through
   serde exactly. -- `tests/report.rs`

## Error model

No `Result` and no panics: every public function is total over all input
values (including inconsistent counts an adapter would never produce).
Degenerate inputs are represented in the data (`meaningful` flags, zeroed
fields), not as errors.

## Determinism class

Fully deterministic and pure: no clocks, no randomness, no IO, no global
state. Float ordering uses `total_cmp` (NaN-total). Identical inputs give
bit-identical outputs; `finalize` is idempotent.

## Cancellation behavior

Synchronous, non-blocking, allocation-bounded by input size; there is
nothing to cancel. A dropped input takes no locks and leaves no residue.

## Unsafe boundary

None. `#![deny(unsafe_code)]` is set at the crate root; no unsafe code
exists.

## Feature flags

None.

## Conformance tests

- `tests/formulas.rs` -- Halstead + MI reference values, degenerate table,
  identity property grid.
- `tests/ranking.rs` -- priority formula, floor, sort/tiebreak/determinism,
  reason clause table.
- `tests/delta.rs` -- path matching, classification table, MI-drop rule,
  duplicate-name pairing, filtering.
- `tests/gates.rs` -- fail-all semantics, median scaling, trivial exemption,
  index order.
- `tests/report.rs` -- from_raw values/rounding, trivial mapping.
- `tests/json.rs` -- codec: escaping round trips, both envelope forms,
  forward-compatible parsing, malformed input (incl. trailing bytes),
  agreement with an independent JSON parser.
- `tests/bands.rs` -- MI band boundaries and report band derivation.
- `tests/common/mod.rs` -- shared fixtures.

Each area was mutation-verified (the suite fails against a deliberate mutant
per module: Stroud constant, dropped nesting factor, epsilon change, gate
`&=` -> `|=`, disabled string escaping, disabled trailing-byte rejection,
band threshold 85 -> 84).

## No-claim boundaries

- `time_seconds` and `estimated_bugs` are order-of-magnitude Halstead
  estimates with limited modern empirical support; use as relative signals
  only.
- `PriorityConfig` defaults are a documented starting point
  (halmi-integration.md), not a calibrated ranking. No benchmark corpus or
  acceptance band exists for them.
- MI thresholds (the 171 normalization, the 50.0 drop threshold) are the
  standard published forms, not validated for any particular codebase.
- `paths_match` suffix matching can conflate same-named files whose paths
  are suffixes of one another; it is a heuristic for root-prefix changes.
