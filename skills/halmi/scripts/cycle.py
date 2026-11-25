#!/usr/bin/env python3
"""Drive one bounded halmi measurement cycle without modifying source reports."""

import argparse
import copy
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="+")
    parser.add_argument("--halmi", default=shutil.which("halmi") or "halmi")
    parser.add_argument("--workdir", type=Path)
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument("--cycles", type=int, default=1)
    parser.add_argument("--top", type=str, default="5")
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--changed-only", action="store_true")
    parser.add_argument("--min-mi", type=float)
    parser.add_argument("--max-difficulty", type=float)
    parser.add_argument("--min-band", choices=["low", "moderate", "high"])
    parser.add_argument("--max-effort", type=float)
    return parser.parse_args()


def halmi_command(args, baseline):
    command = [args.halmi, "--json", "--top", args.top, *args.paths]
    if args.min_mi is not None:
        command += ["--min-mi", str(args.min_mi)]
    if args.max_difficulty is not None:
        command += ["--max-difficulty", str(args.max_difficulty)]
    if args.min_band is not None:
        command += ["--min-band", args.min_band]
    if args.max_effort is not None:
        command += ["--max-effort", str(args.max_effort)]
    if baseline is not None:
        command += ["--baseline", str(baseline)]
        if args.changed_only:
            command += ["--changed-only"]
    return command


def run_halmi(args, baseline):
    """Run halmi; exit codes 0 and 1 are report-bearing, anything else is a
    hard error. halmi's stderr (for example its truncated-baseline warning)
    is passed through instead of being swallowed."""
    result = subprocess.run(halmi_command(args, baseline), cwd=args.workdir, capture_output=True)
    if result.returncode not in [0, 1]:
        raise ValueError(result.stderr.decode("utf-8", "replace"))
    if result.stderr:
        sys.stderr.write(result.stderr.decode("utf-8", "replace"))
    return result.stdout


def run(args):
    if not 1 <= args.cycles <= 3:
        raise ValueError("--cycles must be between 1 and 3")
    if args.workdir and not args.workdir.is_dir():
        raise ValueError(f"workdir is not a directory: {args.workdir}")
    output = args.output_dir or Path(tempfile.mkdtemp(prefix="halmi-cycle-")
                                   )
    output.mkdir(parents=True, exist_ok=True)

    # The baseline must hold every function, not just the --top window:
    # deltas are matched per function, so anything missing from the baseline
    # would be reported as "new" and could never show up as worsened.
    initial_baseline = args.baseline or output / "baseline.json"
    if not initial_baseline.exists():
        capture = copy.copy(args)
        capture.top = "1000000"
        initial_baseline.write_bytes(run_halmi(capture, None))

    existing_cycles = [
        int(p.stem.split("-", 1)[1])
        for p in output.glob("cycle-[0-9][0-9][0-9].json")
    ]
    first_cycle = max(existing_cycles, default=0) + 1
    used_baseline = initial_baseline.resolve()
    for cycle in range(first_cycle, first_cycle + args.cycles):
        stdout = run_halmi(args, used_baseline)
        report = json.loads(stdout)
        destination = output / f"cycle-{cycle:03d}.json"
        destination.write_bytes(stdout)
        summary = report.get("summary", {})
        rows = report.get("results", [])
        findings = [
            {
                "name": row["name"],
                "path": row["path"],
                "start_line": row["start_line"],
                "end_line": row["end_line"],
                "difficulty": row["difficulty"],
                "effort": row["effort"],
                "mi_vs": row["mi_vs"],
                "priority": row["priority"],
                "delta_status": row.get("delta_status"),
            }
            for row in rows[:5]
            if not row.get("delta_status")
            or row.get("delta_status") != "unchanged"
        ]
        gates = summary.get("gates", {})
        payload = {
            "cycle": cycle,
            "artifact": str(destination),
            "analysis_errors": summary.get("errors", []),
            "gate_violation_count": gates.get("violations", 0),
            "gates_configured": bool(gates),
            "delta": summary.get("delta"),
            "findings": findings,
        }
        print(json.dumps(payload, indent=2, sort_keys=True))
        if payload["analysis_errors"] or payload["gate_violation_count"] > 0:
            return 1
    return 0


def main():
    try:
        return run(parse_args())
    except (OSError, ValueError, KeyError, TypeError) as exc:
        print(f"halmi cycle: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
