//! Repository-level aggregations: MI rollup and per-file rollup.

use halmi_core::metrics::MiBand;
use halmi_core::ranking::{is_trivial, median};
use halmi_core::report::FunctionReport;

pub struct MiRollup {
    pub(crate) mi_median: f64,
    pub(crate) high: usize,
    pub(crate) moderate: usize,
    pub(crate) low: usize,
}

pub fn mi_rollup(reports: &[FunctionReport]) -> MiRollup {
    let mut values: Vec<f64> = reports
        .iter()
        .filter(|r| !is_trivial(r))
        .map(|r| r.mi_vs)
        .collect();
    let mi_median = median(&mut values);
    let (mut high, mut moderate, mut low) = (0usize, 0usize, 0usize);
    for r in reports.iter().filter(|r| !is_trivial(r)) {
        match r.mi_band {
            MiBand::High => high += 1,
            MiBand::Moderate => moderate += 1,
            MiBand::Low => low += 1,
        }
    }
    MiRollup {
        mi_median,
        high,
        moderate,
        low,
    }
}

/// One row of the per-file rollup table.
pub struct FileRow {
    pub(crate) path: String,
    pub(crate) functions: usize,
    pub(crate) total_loc: u32,
    pub(crate) total_effort: f64,
    pub(crate) max_difficulty: f64,
    pub(crate) max_priority: f64,
    pub(crate) mean_mi_vs: f64,
    pub(crate) median_mi_vs: f64,
    pub(crate) worst: String,
}

pub fn rollup_by_file(
    reports: &[FunctionReport],
    top: usize,
    sort: crate::args::SortKey,
) -> Vec<FileRow> {
    let mut by_path: std::collections::BTreeMap<&str, Vec<&FunctionReport>> =
        std::collections::BTreeMap::new();
    for r in reports {
        by_path.entry(r.path.as_str()).or_default().push(r);
    }
    let mut rows: Vec<FileRow> = by_path
        .into_iter()
        .map(|(path, fns)| {
            let total_effort: f64 = fns.iter().map(|r| r.effort).sum();
            let max_difficulty = fns.iter().map(|r| r.difficulty).fold(0.0_f64, f64::max);
            let mut mi_values: Vec<f64> = fns
                .iter()
                .filter(|r| !is_trivial(r))
                .map(|r| r.mi_vs)
                .collect();
            let (mean_mi_vs, median_mi_vs) = if mi_values.is_empty() {
                (100.0, 100.0)
            } else {
                let mean =
                    mi_values.iter().sum::<f64>() / halmi_core::report::count_f64(mi_values.len());
                (mean, median(&mut mi_values))
            };
            let worst = fns
                .iter()
                .max_by(|a, b| {
                    a.priority
                        .total_cmp(&b.priority)
                        .then(a.start_line.cmp(&b.start_line))
                })
                .expect("group is non-empty");
            FileRow {
                path: path.to_string(),
                functions: fns.len(),
                total_loc: fns.iter().map(|r| r.loc).sum(),
                total_effort,
                max_difficulty,
                max_priority: worst.priority,
                mean_mi_vs,
                median_mi_vs,
                worst: format!("{}@{}", worst.name, worst.start_line),
            }
        })
        .collect();
    rows.sort_by(|a, b| match sort {
        crate::args::SortKey::Priority => b
            .max_priority
            .total_cmp(&a.max_priority)
            .then(b.total_effort.total_cmp(&a.total_effort))
            .then(a.path.cmp(&b.path)),
        crate::args::SortKey::Effort => b
            .total_effort
            .total_cmp(&a.total_effort)
            .then(a.path.cmp(&b.path)),
        crate::args::SortKey::Difficulty => b
            .max_difficulty
            .total_cmp(&a.max_difficulty)
            .then(a.path.cmp(&b.path)),
        crate::args::SortKey::Mi => a
            .mean_mi_vs
            .partial_cmp(&b.mean_mi_vs)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.path.cmp(&b.path)),
        crate::args::SortKey::Cc | crate::args::SortKey::Loc => {
            b.total_loc.cmp(&a.total_loc).then(a.path.cmp(&b.path))
        }
    });
    rows.truncate(top);
    rows
}
