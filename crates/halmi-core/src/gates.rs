//! Joint quality gates: compound conditions that decide the CLI exit code.
//!
//! A function "violates the gates" only when it fails *every* enabled gate.
//! Requiring all enabled gates to fail keeps false positives down compared
//! with flagging on any single metric (see halmi-integration.md section 5).

use crate::ranking::is_trivial;
use crate::report::FunctionReport;

/// Enabled joint thresholds. Any subset may be set.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Gates {
    /// Fail when `mi_vs < min_mi`.
    pub min_mi: Option<f64>,
    /// Fail when `difficulty > max_difficulty`.
    pub max_difficulty: Option<f64>,
    /// Fail when `effort > max_effort_factor * median_effort`.
    pub max_effort_factor: Option<f64>,
    /// Fail when the MI band is worse (lower) than `min_band`
    /// (Low < Moderate < High).
    pub min_band: Option<crate::metrics::MiBand>,
}

impl Gates {
    #[must_use]
    pub const fn any_enabled(&self) -> bool {
        self.min_mi.is_some()
            || self.max_difficulty.is_some()
            || self.max_effort_factor.is_some()
            || self.min_band.is_some()
    }
}

/// True when `r` fails all enabled gates (and at least one is enabled).
/// Trivial functions never violate gates.
#[must_use]
pub fn violates_all(gates: &Gates, r: &FunctionReport, median_effort: f64) -> bool {
    if !gates.any_enabled() || is_trivial(r) {
        return false;
    }
    let mut failed_all = true;
    if let Some(min_mi) = gates.min_mi {
        failed_all &= r.mi_vs < min_mi;
    }
    if let Some(max_difficulty) = gates.max_difficulty {
        failed_all &= r.difficulty > max_difficulty;
    }
    if let Some(factor) = gates.max_effort_factor {
        failed_all &= r.effort > factor * median_effort.max(1.0);
    }
    if let Some(min_band) = gates.min_band {
        failed_all &= r.mi_band < min_band;
    }
    failed_all
}

/// Indices of reports that violate the joint gates, in report order.
#[must_use]
pub fn violations(gates: &Gates, reports: &[FunctionReport], median_effort: f64) -> Vec<usize> {
    reports
        .iter()
        .enumerate()
        .filter(|(_, r)| violates_all(gates, r, median_effort))
        .map(|(i, _)| i)
        .collect()
}
