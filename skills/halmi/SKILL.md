---
name: halmi
description: Run a bounded measure-fix-verify cycle with halmi. Use when reviewing or refactoring code with halmi JSON reports, prioritizing Halstead/Maintainability hotspots, comparing against an immutable baseline, or asked to keep fixing until chosen gates and tests are green. Not for cccc-only complexity rankings or unvalidated claims of universal maintainability.
---

# halmi cycle

Use halmi itself for focused changes with direct evidence. Do not use a lower
ranking by itself as permission to delete, rewrite, or move code blindly.

## Preconditions

- A `halmi` binary is available on `PATH`, or pass its full location with
  `--halmi /absolute/path/to/halmi`.
- Python 3 with only its standard library is available.
- Read the target project's metric definitions and test contracts, plus
  `docs/SCIENCE.md` when present.
## Binary provenance

Refresh the binary after changing the analyzer or toolchain, then capture data
with that refreshed build. Do not trust cached reports or `--version` alone: it
identifies the package version, not which source checkout produced it.

## Measurement contract

Preserve one immutable baseline for a change series. Record each cycle in a new
artifact instead of overwriting it or rebasing merely to erase a warning.

The completion condition is all of these, not merely an empty top-N list:

1. No analysis errors are reported.
2. The explicitly selected gates have zero violations, if any were selected.
3. Relevant baseline names/spans are intentional; every removal was deliberate,
   not an accidental way to silence a metric.
4. The meaningful delta pass neither shows worsened priority on target
   functions nor loses MI with increased effort on the same key functions.
5. The target's own required tests and differentials, if applicable, pass.

No gate set is universal. Never activate arbitrary gates just to force red or
green output. If no gates were selected, say that explicitly. Gate predicates
are conjunctive: a violation only when every enabled gate fails. Passing any one
enabled floor clears that invocation. Thus a permissive band can weaken another
threshold; adding `min-band low` alone is not a meaningful strictness goal.


## Typical invocation

From the repository that contains this skill:

```bash
python3 skills/halmi/scripts/cycle.py crates/foo/src --top 5 \
  --output-dir /tmp/halmi-cycle-foo --min-mi 40 --max-difficulty 30
```

Use `--halmi` for another binary, `--cycles` to change the default of one cycle,
and `--changed-only` to focus output on the original baseline's worst deltas
(it works from the first run: the baseline is captured before it is applied).
The baseline holds every function in the scanned paths, not just the `--top`
window, so a function of any rank can surface as worsened; `--top` only limits
what each cycle prints. Existing artifact directories are resumed without
changing the baseline. Use a
new artifact directory when the target, source root, language profile, or gate
configuration changes; do not accidentally compare unrelated snapshots.

## Model procedure

1. Read the emitted cycle JSON. For a semantic defect, first express the failure
   in a focused existing test or oracle and preserve that failing-first run.
   Then make one behavior-preserving change at one
   defensible source location. Prefer removing a named defect, deduplication,
   or a clear error guard over shuffling the same decision tree to game stats.
2. Run the target's own focused test lane and check for parse failures. Never
   waive a distinct defect because another metric went down.
3. Run the same `cycle.py` command again. Compare effort, difficulty, MI, CC,
   priority, names, and spans. Inspect mixed results: effort may go down while
   mean or median MI falls.
4. If still outside the selected limits, repeat with the next smallest sound
   change. Stop after three fix attempts and state the evidence and next blocker.
5. Only call the result green when that completion condition holds. Cite the
   cycle artifacts and test commands. With no configured gates, call it a delta
   report, not an unconditional quality score.

For a second opinion, use `cccc` independently for cognitive and cyclomatic
complexity. That does not validate halmi's Halstead or MI accounting.

## Guardrails

- Never delete or rewrite baselines, replace reports, or weaken tests to claim
  convergence.
- Do not regenerate output under a different normalization or language profile
  to make deltas disappear.
- Do not modify other packages through broad test commands when the work calls
  for a focused lane.
- Never create or push branches, pull requests, releases, or external messages
  on the user's behalf.
- Do not auto-edit or delete unrelated files. Never remove a file to eliminate a
  metric. A delete requires explicit owner permission.

## Results to report

- What changed and why.
- Per-function before/after difficulty, effort, MI, and priority.
- Whole-file maximum difficulty, total effort, mean/median MI, and any mixed MI
  regressions.
- The exact focused tests and differentials with exits, plus artifact paths.
- Remaining gaps: unknown syntax, classification conventions, missing oracle,
  uncalibrated thresholds, or anything not independently verified.
