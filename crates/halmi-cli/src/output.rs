//! Output rendering: the JSON envelope, SARIF 2.1.0, and the tables.

use std::collections::HashSet;

use crate::rollup::FileRow;
use halmi_core::gates::Gates;
use halmi_core::json::{write_f64, write_string};
use halmi_core::ranking::{RepoStats, is_trivial};
use halmi_core::report::{FunctionReport, SCHEMA_VERSION, write_report_json};

use crate::args::Color;
use crate::rollup::MiRollup;

pub struct ErrorEntry {
    pub(crate) path: String,
    pub(crate) message: String,
}

pub struct DeltaSummary {
    pub(crate) baseline: String,
    pub(crate) new: usize,
    pub(crate) worsened: usize,
    pub(crate) improved: usize,
    pub(crate) unchanged: usize,
}

/// One field per top-level JSON section, in output order.
pub struct JsonEnvelope<'a> {
    pub(crate) root: &'a str,
    pub(crate) files: usize,
    pub(crate) functions: usize,
    pub(crate) stats: &'a RepoStats,
    pub(crate) rollup: &'a MiRollup,
    pub(crate) errors: &'a [ErrorEntry],
    pub(crate) delta: Option<&'a DeltaSummary>,
    pub(crate) gates: &'a Gates,
    pub(crate) gate_violations: usize,
    pub(crate) violators: &'a [String],
    pub(crate) language_counts: &'a [(halmi_core::report::Language, usize)],
    pub(crate) results: &'a [FunctionReport],
    pub(crate) file_rows: Option<&'a [FileRow]>,
}

pub fn write_output_json(out: &mut String, envelope: &JsonEnvelope<'_>) {
    out.push_str("{\n  \"schema_version\": ");
    let _ = std::fmt::Write::write_fmt(out, format_args!("{SCHEMA_VERSION}"));
    out.push_str(",\n  \"root\": ");
    write_string(out, envelope.root);
    write_summary(out, envelope);
    if envelope.gates.any_enabled() {
        write_gates(out, envelope);
    }
    out.push_str("\n  },\n  \"results\": [");
    for (i, r) in envelope.results.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_report_json(out, r);
    }
    out.push(']');
    if let Some(rows) = envelope.file_rows {
        write_file_rows(out, rows);
    }
    out.push_str("\n}\n");
}

/// The `"summary"` object: counts, medians, MI bands, languages, errors, delta.
fn write_summary(out: &mut String, e: &JsonEnvelope<'_>) {
    out.push_str(",\n  \"summary\": {\n    \"files\": ");
    let _ = std::fmt::Write::write_fmt(out, format_args!("{}", e.files));
    out.push_str(",\n    \"functions\": ");
    let _ = std::fmt::Write::write_fmt(out, format_args!("{}", e.functions));
    out.push_str(",\n    \"median_effort\": ");
    write_f64(out, e.stats.median_effort);
    out.push_str(",\n    \"mi_median\": ");
    write_f64(out, e.rollup.mi_median);
    out.push_str(",\n    \"mi_bands\": {\"high\": ");
    let _ = std::fmt::Write::write_fmt(
        out,
        format_args!(
            "{}, \"moderate\": {}, \"low\": {}",
            e.rollup.high, e.rollup.moderate, e.rollup.low
        ),
    );
    out.push('}');
    out.push_str(",\n    \"languages\": {");
    for (i, (lang, n)) in e.language_counts.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write_string(out, lang.as_str());
        out.push_str(": ");
        let _ = std::fmt::Write::write_fmt(out, format_args!("{n}"));
    }
    out.push('}');
    if !e.errors.is_empty() {
        out.push_str(",\n    \"errors\": [");
        for (i, err) in e.errors.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"path\": ");
            write_string(out, &err.path);
            out.push_str(", \"message\": ");
            write_string(out, &err.message);
            out.push('}');
        }
        out.push(']');
    }
    if let Some(d) = e.delta {
        out.push_str(",\n    \"delta\": {\"baseline\": ");
        write_string(out, &d.baseline);
        let _ = std::fmt::Write::write_fmt(
            out,
            format_args!(
                ", \"new\": {}, \"worsened\": {}, \"improved\": {}, \"unchanged\": {}}}",
                d.new, d.worsened, d.improved, d.unchanged
            ),
        );
    }
}

/// The `"gates"` object, present only when at least one gate is enabled.
fn write_gates(out: &mut String, e: &JsonEnvelope<'_>) {
    out.push_str(",\n    \"gates\": {\"min_mi\": ");
    match e.gates.min_mi {
        Some(v) => write_f64(out, v),
        None => out.push_str("null"),
    }
    out.push_str(", \"max_difficulty\": ");
    match e.gates.max_difficulty {
        Some(v) => write_f64(out, v),
        None => out.push_str("null"),
    }
    out.push_str(", \"max_effort_factor\": ");
    match e.gates.max_effort_factor {
        Some(v) => write_f64(out, v),
        None => out.push_str("null"),
    }
    out.push_str(", \"min_band\": ");
    match e.gates.min_band {
        Some(b) => write_string(out, b.as_str()),
        None => out.push_str("null"),
    }
    let _ = std::fmt::Write::write_fmt(
        out,
        format_args!(", \"violations\": {}, \"violators\": [", e.gate_violations),
    );
    for (i, v) in e.violators.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_string(out, v);
    }
    out.push_str("]}");
}

/// The top-level `"files"` array of the `--by-file` rollup.
fn write_file_rows(out: &mut String, rows: &[FileRow]) {
    out.push_str(",\n  \"files\": [");
    for (i, row) in rows.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"path\": ");
        write_string(out, &row.path);
        let _ = std::fmt::Write::write_fmt(
            out,
            format_args!(
                ", \"functions\": {}, \"total_loc\": {}, \"total_effort\": ",
                row.functions, row.total_loc
            ),
        );
        write_f64(out, row.total_effort);
        out.push_str(", \"max_difficulty\": ");
        write_f64(out, row.max_difficulty);
        out.push_str(", \"max_priority\": ");
        write_f64(out, row.max_priority);
        out.push_str(", \"mean_mi_vs\": ");
        write_f64(out, row.mean_mi_vs);
        out.push_str(", \"median_mi_vs\": ");
        write_f64(out, row.median_mi_vs);
        out.push_str(", \"worst\": ");
        write_string(out, &row.worst);
        out.push('}');
    }
    out.push(']');
}

pub fn color_enabled(mode: Color) -> bool {
    match mode {
        Color::Always => true,
        Color::Never => false,
        Color::Auto => std::io::IsTerminal::is_terminal(&std::io::stdout()),
    }
}

pub const RED: &str = "\u{1b}[31m";
pub const YELLOW: &str = "\u{1b}[33m";
pub const RESET: &str = "\u{1b}[0m";

pub fn heat_color(cell: &str, rank: usize, total: usize, colored: bool) -> String {
    if !colored || total == 0 {
        return cell.to_string();
    }
    let quartile = rank * 4 / total;
    match quartile {
        0 => format!("{RED}{cell}{RESET}"),
        1 => format!("{YELLOW}{cell}{RESET}"),
        _ => cell.to_string(),
    }
}

pub fn render_file_table(rows: &[FileRow], colored: bool) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:>10} {:>7} {:>6} {:>12} {:>8} {:>8} {:>8} {:<30} PATH",
        "MAXPRI", "FUNCS", "LOC", "EFFORT", "MAX_D", "MEAN_MI", "MED_MI", "WORST"
    );
    for (rank, r) in rows.iter().enumerate() {
        let pri = format!("{:.1}", r.max_priority);
        let _ = writeln!(
            out,
            "{:>10} {:>7} {:>6} {:>12.1} {:>8.1} {:>8.1} {:>8.1} {:<30} {}",
            heat_color(&pri, rank, rows.len(), colored),
            r.functions,
            r.total_loc,
            r.total_effort,
            r.max_difficulty,
            r.mean_mi_vs,
            r.median_mi_vs,
            truncate(&r.worst, 30),
            r.path,
        );
    }
    out
}

pub const SARIF_SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";
pub const SARIF_HOTSPOT_RULE: &str = "HALMI001";

/// SARIF 2.1.0 output: one result per non-trivial emitted function so CI and
/// agent SARIF consumers can consume halmi hotspots directly. Gate
/// violations are level "warning"; everything else is "note". Trivial
/// functions are excluded: they carry no diagnostic signal.
pub fn render_sarif(results: &[FunctionReport], gate_violation_indices: &[usize]) -> String {
    let violating: HashSet<usize> = gate_violation_indices.iter().copied().collect();
    let mut out = String::new();
    out.push_str("{\n  \"$schema\": ");
    write_string(&mut out, SARIF_SCHEMA);
    out.push_str(",\n  \"version\": \"2.1.0\",\n  \"runs\": [{\n    \"tool\": {\n      \"driver\": {\n        \"name\": \"halmi\",\n        \"version\": \"");
    out.push_str(env!("CARGO_PKG_VERSION"));
    out.push_str("\",\n        \"rules\": [{\n          \"id\": ");
    write_string(&mut out, SARIF_HOTSPOT_RULE);
    out.push_str(",\n          \"shortDescription\": {\"text\": ");
    write_string(&mut out, "Joint Halstead + Maintainability Index hotspot");
    out.push_str("}\n        }]\n      }\n    },\n    \"results\": [");
    let mut first = true;
    for (i, r) in results.iter().enumerate().filter(|(_, r)| !is_trivial(r)) {
        if !first {
            out.push(',');
        }
        first = false;
        let level = if violating.contains(&i) {
            "warning"
        } else {
            "note"
        };
        let message = format!(
            "{}: {} (priority {:.1}, MI {:.1}, effort {:.1}, difficulty {:.1}, CC {}, LOC {})",
            r.name, r.reason, r.priority, r.mi_vs, r.effort, r.difficulty, r.cyclomatic, r.loc
        );
        out.push_str("\n      {\n        \"ruleId\": ");
        write_string(&mut out, SARIF_HOTSPOT_RULE);
        out.push_str(",\n        \"level\": ");
        write_string(&mut out, level);
        out.push_str(",\n        \"message\": {\"text\": ");
        write_string(&mut out, &message);
        out.push_str("},\n        \"locations\": [{\n          \"physicalLocation\": {\n            \"artifactLocation\": {\"uri\": ");
        write_string(&mut out, &r.path);
        out.push_str("},\n            \"region\": {\"startLine\": ");
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!("{}, \"endLine\": {}}}", r.start_line, r.end_line),
        );
        out.push_str("\n          }\n        }]\n      }");
    }
    out.push_str("\n    ]\n  }]\n}\n");
    out
}

pub fn truncate(s: &str, n: usize) -> &str {
    if s.len() <= n || n == 0 {
        s
    } else {
        let mut end = n - 1;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

pub fn render_table(results: &[FunctionReport], colored: bool) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:>10} {:>6} {:>6} {:>10} {:>3} {:>4} {:>4}  {:<28} LOCATION",
        "PRIORITY", "MI", "DIFF", "EFFORT", "CC", "LOC", "NST", "NAME"
    );
    for (rank, r) in results.iter().enumerate() {
        let pri = format!("{:.2}", r.priority);
        let _ = writeln!(
            out,
            "{:>10} {:>6.1} {:>6.1} {:>10.1} {:>3} {:>4} {:>4}  {:<28} {}:{}-{}",
            heat_color(&pri, rank, results.len(), colored),
            r.mi_vs,
            r.difficulty,
            r.effort,
            r.cyclomatic,
            r.loc,
            r.nesting,
            truncate(&r.name, 28),
            r.path,
            r.start_line,
            r.end_line,
        );
    }
    out
}
