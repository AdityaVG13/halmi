//! Joint priority scoring, reason strings, and repository-level ranking.

use crate::report::FunctionReport;

/// Tunable weights for the priority formula. Defaults follow the starting
/// formula in docs/internal/halmi-integration.md; tune empirically.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriorityConfig {
    /// Multiplier per nesting level above `nest_free`.
    pub nest_weight: f64,
    /// Nesting levels allowed before the penalty kicks in.
    pub nest_free: f64,
    /// Weight of `ln(loc)` in the size factor.
    pub size_weight: f64,
    /// Floor for the repository median effort (avoids tiny medians
    /// exploding the effort ratio).
    pub effort_floor: f64,
}

impl Default for PriorityConfig {
    fn default() -> Self {
        Self {
            nest_weight: 0.15,
            nest_free: 2.0,
            size_weight: 0.2,
            effort_floor: 1.0,
        }
    }
}

/// Repository-level stats computed during [`finalize`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RepoStats {
    /// Median effort across non-trivial functions (0.0 when none).
    pub median_effort: f64,
    /// Number of non-trivial functions.
    pub meaningful: usize,
}

/// True when the function is too small for metrics to mean anything.
/// Mirrors the tiny-function rule in halmi-formulas.md.
#[must_use]
pub fn is_trivial(r: &FunctionReport) -> bool {
    r.volume <= 0.0 || r.loc == 0
}

/// The joint priority score. Higher = fix first.
///
/// ```text
/// priority = (effort / median_effort)
///          * (100 - mi_vs)
///          * (1 + nest_weight * max(0, nesting - nest_free))
///          * (1 + size_weight * ln(loc))
/// ```
#[must_use]
#[allow(
    clippy::suboptimal_flops,
    reason = "the published formulas are plain a*b + c sequences; fused multiply-add rounds differently and would break bit-exact agreement with the reference values and recorded baselines"
)]
pub fn compute_priority(
    cfg: &PriorityConfig,
    effort: f64,
    mi_vs: f64,
    nesting: u32,
    loc: u32,
    median_effort: f64,
) -> f64 {
    let effort_ratio = effort / median_effort.max(cfg.effort_floor);
    let mi_penalty = (100.0 - mi_vs).max(0.0);
    let nest_factor = 1.0 + cfg.nest_weight * (f64::from(nesting) - cfg.nest_free).max(0.0);
    let size_factor = 1.0 + cfg.size_weight * f64::from(loc).ln().max(0.0);
    effort_ratio * mi_penalty * nest_factor * size_factor
}

/// A short, actionable explanation derived from the combined signals.
/// Returns at most two clauses joined by `"; "`.
#[must_use]
pub fn reason_for(r: &FunctionReport, median_effort: f64) -> String {
    if is_trivial(r) {
        return "trivial function".into();
    }
    let mut first: Option<&'static str> = None;
    let mut second: Option<&'static str> = None;
    macro_rules! push {
        ($c:expr) => {{
            let c: &'static str = $c;
            if first.is_none() {
                first = Some(c);
            } else if second.is_none() {
                second = Some(c);
            }
        }};
    }
    if r.effort > 2.0 * median_effort.max(1.0) && r.mi_vs < 50.0 {
        push!("dense and large -- extract method or introduce intermediates");
    }
    if r.method_chain >= 4 && r.difficulty > 20.0 {
        push!("long method chain -- idiomatic but dense; consider binding intermediates");
    } else if r.difficulty > 30.0 && r.cyclomatic < 10 {
        push!("dense expressions rather than control flow -- simplify arithmetic or naming");
    }
    if r.purity_ratio < 0.7 && r.loc > 20 {
        push!("repeated patterns -- consider extraction");
    }
    if r.estimated_bugs > 1.0 {
        push!("high information content with elevated defect signal");
    }
    if r.nesting > 4 {
        push!("deep nesting -- flatten control flow");
    }
    if r.cyclomatic > 15 {
        push!("high branching -- split or use early returns");
    }
    if first.is_none() && r.mi_band == crate::metrics::MiBand::Low {
        push!("low maintainability index");
    }
    match (first, second) {
        (Some(a), Some(b)) => format!("{a}; {b}"),
        (Some(a), None) => a.into(),
        (None, _) => "no dominant signal".into(),
    }
}

pub fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values.select_nth_unstable_by(mid, f64::total_cmp);
        values[mid]
    } else {
        values.select_nth_unstable_by(mid, f64::total_cmp);
        let hi = values[mid];
        let lo = values[..mid]
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        f64::midpoint(lo, hi)
    }
}

/// Fill in `priority` and `reason` for every report using repository medians,
/// then sort by priority (descending), effort (descending), and location
/// (ascending) for a stable order.
pub fn finalize(reports: &mut [FunctionReport], cfg: &PriorityConfig) -> RepoStats {
    finalize_impl(reports, cfg, None)
}

impl FunctionReport {
    /// Fill in `priority` and `reason` against the repository median effort;
    /// trivial functions rank at 0.
    pub(crate) fn apply_rank(&mut self, cfg: &PriorityConfig, med: f64) {
        self.priority = if is_trivial(self) {
            0.0
        } else {
            let p = compute_priority(cfg, self.effort, self.mi_vs, self.nesting, self.loc, med);
            (p * 1000.0).round() / 1000.0
        };
        self.reason = reason_for(self, med);
    }
}

/// [`finalize`], but only the first `top` reports are guaranteed to be
/// in order (a partial selection instead of a full sort).
pub fn finalize_top(reports: &mut [FunctionReport], cfg: &PriorityConfig, top: usize) -> RepoStats {
    finalize_impl(reports, cfg, Some(top))
}

fn finalize_impl(
    reports: &mut [FunctionReport],
    cfg: &PriorityConfig,
    top: Option<usize>,
) -> RepoStats {
    let mut efforts: Vec<f64> = reports
        .iter()
        .filter(|r| !is_trivial(r))
        .map(|r| r.effort)
        .collect();
    let med = median(&mut efforts);
    for r in reports.iter_mut() {
        r.apply_rank(cfg, med);
    }
    let cmp = |a: &FunctionReport, b: &FunctionReport| {
        b.priority
            .total_cmp(&a.priority)
            .then(b.effort.total_cmp(&a.effort))
            .then((a.path.as_str(), a.name.as_str(), a.start_line).cmp(&(
                b.path.as_str(),
                b.name.as_str(),
                b.start_line,
            )))
    };
    match top {
        Some(t) if t < reports.len() => {
            reports.select_nth_unstable_by(t, cmp);
            reports[..=t].sort_by(|a, b| cmp(a, b));
        }
        _ => reports.sort_by(|a, b| cmp(a, b)),
    }
    RepoStats {
        median_effort: med,
        meaningful: efforts.len(),
    }
}
