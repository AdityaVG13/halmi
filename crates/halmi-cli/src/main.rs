//! halmi: agent-native Halstead + Maintainability Index analyzer.
//!
//! Thin binary over halmi-core and the language adapters: discovers source
//! files, analyzes them in parallel with a std-thread pool, ranks by joint
//! priority, and emits stable JSON (primary), a human table, or SARIF 2.1.0.
//! Baseline delta mode and joint threshold gates set the exit code. Zero
//! external dependencies: JSON emission and parsing use halmi-core's codec.

#![deny(unsafe_code)]

mod analysis;
mod args;
mod config;
mod output;
mod rollup;
mod walk;

use std::path::Path;
use std::process::ExitCode;

use halmi_core::adapter::SourceAnalysis;
use halmi_core::report::FunctionReport;

use halmi_core::delta::{DeltaStats, attach_delta, retain_worsened};
use halmi_core::gates::{Gates, violations};
use halmi_core::ranking::{PriorityConfig, RepoStats, finalize_top};

use crate::analysis::{analyze_files, load_baseline};
use crate::args::{Args, Format, Parsed, SortKey, parse_args};
use crate::output::{
    DeltaSummary, ErrorEntry, JsonEnvelope, color_enabled, render_file_table, render_sarif,
    render_table, write_output_json,
};
use crate::rollup::{FileRow, MiRollup, mi_rollup, rollup_by_file};
use crate::walk::collect_source_files;

pub(crate) const USAGE: &str =
    "halmi -- joint Halstead + Maintainability Index analysis, ranked by priority

USAGE:
    halmi [OPTIONS] [PATHS...]        (defaults to the current directory)

OUTPUT:
    --format <json|table|sarif>   output format (default: json)
    --json                  alias for --format json
    --top <N>               only the N highest-priority functions (default 20)
    --exclude <GLOB>        skip files whose path matches (* and ?); repeatable
    --sort <KEY>            order results by priority|effort|difficulty|mi|cc|loc
    --min-priority <V>      only functions with priority >= V
    --min-effort <V>        Halstead display floors (effort/difficulty/volume/loc)
    --min-difficulty <V>
    --min-volume <V>
    --min-loc <V>
    --config <FILE>         config file (default ./halmi.toml when present)
    --by-file               per-file rollup (json/table; not with --baseline)
    --color <MODE>          table heat coloring: auto|always|never (default auto)

DELTA:
    --baseline <FILE>       compare against a previous halmi JSON run
    --changed-only          with --baseline: only functions that got worse

GATES (exit code 1 when a function fails ALL enabled gates):
    --min-mi <V>            maintainability floor (mi_vs)
    --max-difficulty <V>    Halstead difficulty ceiling
    --max-effort <F>        effort ceiling as a multiple of the repo median
    --min-band <B>          MI band floor: high, moderate, or low

OTHER:
    -h, --help              show this help
    -V, --version           show version";

fn apply_delta(
    reports: &mut Vec<FunctionReport>,
    baseline_path: &Path,
    changed_only: bool,
) -> Result<DeltaSummary, String> {
    let baseline = load_baseline(baseline_path)?;
    let DeltaStats {
        new,
        worsened,
        improved,
        unchanged,
    } = attach_delta(reports, &baseline);
    let summary = DeltaSummary {
        baseline: baseline_path.display().to_string(),
        new,
        worsened,
        improved,
        unchanged,
    };
    let total = new + worsened + improved + unchanged;
    if total > 0 && new * 10 > total {
        eprintln!(
            "halmi: warning: {new} of {total} functions are new vs the baseline (baseline holds {} functions); a truncated baseline (--top) or a different tree root produces this",
            baseline.len(),
        );
    }
    if changed_only {
        retain_worsened(reports);
    }
    Ok(summary)
}

fn evaluate_gates(
    gates: &Gates,
    reports: &[FunctionReport],
    median_effort: f64,
) -> (usize, Vec<String>, Vec<usize>) {
    let indices: Vec<usize> = if gates.any_enabled() {
        violations(gates, reports, median_effort)
    } else {
        Vec::new()
    };
    let violators = indices
        .iter()
        .map(|&i| {
            format!(
                "{}@{}:{}",
                reports[i].name, reports[i].path, reports[i].start_line
            )
        })
        .collect();
    (indices.len(), violators, indices)
}

fn language_counts(reports: &[FunctionReport]) -> [(halmi_core::report::Language, usize); 16] {
    use halmi_core::report::Language;
    let mut counts = [
        (Language::Rust, 0usize),
        (Language::Python, 0),
        (Language::Go, 0),
        (Language::Swift, 0),
        (Language::Zig, 0),
        (Language::C, 0),
        (Language::Cpp, 0),
        (Language::Java, 0),
        (Language::Cs, 0),
        (Language::Kotlin, 0),
        (Language::Php, 0),
        (Language::Ruby, 0),
        (Language::Lua, 0),
        (Language::JavaScript, 0),
        (Language::TypeScript, 0),
        (Language::Other, 0),
    ];
    for r in reports {
        for (lang, n) in &mut counts {
            if *lang == r.language {
                *n += 1;
            }
        }
    }
    counts
}

fn fold_analyses(analyses: &[SourceAnalysis]) -> (Vec<ErrorEntry>, Vec<FunctionReport>) {
    let mut errors = Vec::new();
    let mut reports = Vec::new();
    for a in analyses {
        match &a.error {
            Some(msg) => errors.push(ErrorEntry {
                path: a.path.clone(),
                message: msg.clone(),
            }),
            None => reports.extend(a.functions.iter().cloned()),
        }
    }
    (errors, reports)
}

/// Inputs every output format shares once ranking, delta, and the gates ran.
struct Computed<'a> {
    files: usize,
    functions: usize,
    stats: &'a RepoStats,
    rollup: &'a MiRollup,
    errors: &'a [ErrorEntry],
    delta: Option<&'a DeltaSummary>,
    language_counts: &'a [(halmi_core::report::Language, usize)],
    gate_violations: usize,
    violators: &'a [String],
    gate_indices: &'a [usize],
}

impl Computed<'_> {
    const fn envelope<'a>(
        &'a self,
        root: &'a str,
        gates: &'a Gates,
        results: &'a [FunctionReport],
        file_rows: Option<&'a [FileRow]>,
    ) -> JsonEnvelope<'a> {
        JsonEnvelope {
            root,
            files: self.files,
            functions: self.functions,
            stats: self.stats,
            rollup: self.rollup,
            errors: self.errors,
            delta: self.delta,
            gates,
            gate_violations: self.gate_violations,
            violators: self.violators,
            language_counts: self.language_counts,
            results,
            file_rows,
        }
    }
}

/// Re-rank by a non-default `--sort` key, keeping the `top` best rows first.
fn sort_reports(reports: &mut [FunctionReport], key: SortKey, top: usize) {
    let cmp = |a: &FunctionReport, b: &FunctionReport| {
        let (va, vb) = (key.value(a), key.value(b));
        let ord = if key.ascending() {
            va.total_cmp(&vb)
        } else {
            vb.total_cmp(&va)
        };
        ord.then((a.path.as_str(), a.name.as_str(), a.start_line).cmp(&(
            b.path.as_str(),
            b.name.as_str(),
            b.start_line,
        )))
    };
    let t = top.min(reports.len().saturating_sub(1));
    reports.select_nth_unstable_by(t, cmp);
    reports[..=t].sort_by(|a, b| cmp(a, b));
}

/// Drop functions below the `--min-*` floors.
fn apply_min_filters(reports: &mut Vec<FunctionReport>, args: &Args) {
    if let Some(floor) = args.min_priority {
        reports.retain(|r| r.priority >= floor);
    }
    if let Some(floor) = args.min_effort {
        reports.retain(|r| r.effort >= floor);
    }
    if let Some(floor) = args.min_difficulty {
        reports.retain(|r| r.difficulty >= floor);
    }
    if let Some(floor) = args.min_volume {
        reports.retain(|r| r.volume >= floor);
    }
    if let Some(floor) = args.min_loc {
        reports.retain(|r| f64::from(r.loc) >= floor);
    }
}

/// Per-file rollup (`--by-file`); exit 1 when any function fails the gates.
fn run_by_file(args: &Args, reports: &[FunctionReport], done: &Computed<'_>) -> u8 {
    let rows = rollup_by_file(reports, args.top, args.sort);
    match args.format {
        Format::Json => {
            let root = args.paths.join(", ");
            let mut out = String::new();
            write_output_json(
                &mut out,
                &done.envelope(&root, &args.gates, &[], Some(&rows)),
            );
            println!("{out}");
        }
        Format::Table => println!("{}", render_file_table(&rows, color_enabled(args.color))),
        Format::Sarif => unreachable!("rejected at parse"),
    }
    if done.gate_violations > 0 {
        eprintln!(
            "halmi: {} function(s) violated the joint gates",
            done.gate_violations
        );
        return 1;
    }
    0
}

/// Per-function output; exit 1 when any function fails the gates.
fn run_functions(args: &Args, reports: Vec<FunctionReport>, done: &Computed<'_>) -> u8 {
    let results: Vec<FunctionReport> = reports.into_iter().take(args.top).collect();
    match args.format {
        Format::Json => {
            let root = args.paths.join(", ");
            let mut out = String::new();
            write_output_json(&mut out, &done.envelope(&root, &args.gates, &results, None));
            println!("{out}");
        }
        Format::Table => println!("{}", render_table(&results, color_enabled(args.color))),
        Format::Sarif => println!("{}", render_sarif(&results, done.gate_indices)),
    }
    if done.gate_violations > 0 {
        let val = |v: Option<f64>| v.map_or_else(|| "-".to_string(), |v| format!("{v}"));
        let band = args
            .gates
            .min_band
            .map_or_else(|| "-".to_string(), |b| b.as_str().to_string());
        eprintln!(
            "halmi: {} function(s) violated the joint gates (min-mi={} max-difficulty={} max-effort={} min-band={})",
            done.gate_violations,
            val(args.gates.min_mi),
            val(args.gates.max_difficulty),
            val(args.gates.max_effort_factor),
            band,
        );
        return 1;
    }
    0
}

/// Exit status as a process code: 0 clean, 1 gate violation.
fn run() -> Result<u8, String> {
    let (mut args, explicit, config_path) = match parse_args(std::env::args().skip(1))? {
        Parsed::Args(a, e, c) => (a, e, c),
        Parsed::Help => {
            println!("{USAGE}");
            return Ok(0);
        }
        Parsed::Version => {
            println!("halmi {}", env!("CARGO_PKG_VERSION"));
            return Ok(0);
        }
    };

    config::apply_config(&mut args, &explicit, config_path)?;

    let files = collect_source_files(&args.paths, &args.excludes);
    if files.is_empty() {
        eprintln!(
            "halmi: no source files found under {}",
            args.paths.join(", ")
        );
        return Ok(0);
    }

    let analyses = analyze_files(&files);
    let (errors, mut reports) = fold_analyses(&analyses);

    let stats = finalize_top(&mut reports, &PriorityConfig::default(), args.top);
    if args.sort != SortKey::Priority && !reports.is_empty() {
        sort_reports(&mut reports, args.sort, args.top);
    }
    let rollup = mi_rollup(&reports);
    let language_counts = language_counts(&reports);

    let delta_summary = args
        .baseline
        .as_ref()
        .map(|baseline_path| apply_delta(&mut reports, baseline_path, args.changed_only))
        .transpose()?;
    let (gate_violations, violators, gate_indices) =
        evaluate_gates(&args.gates, &reports, stats.median_effort);
    let done = Computed {
        files: files.len(),
        functions: reports.len(),
        stats: &stats,
        rollup: &rollup,
        errors: &errors,
        delta: delta_summary.as_ref(),
        language_counts: &language_counts,
        gate_violations,
        violators: &violators,
        gate_indices: &gate_indices,
    };
    apply_min_filters(&mut reports, &args);

    if args.by_file {
        return Ok(run_by_file(&args, &reports, &done));
    }
    Ok(run_functions(&args, reports, &done))
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(msg) => {
            eprintln!("halmi: {msg}");
            ExitCode::from(2)
        }
    }
}
