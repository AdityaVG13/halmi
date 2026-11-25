//! Baseline / delta mode: compare a current run against a stored run and
//! flag functions that got worse.

use std::collections::HashMap;

use crate::report::{DeltaInfo, DeltaStatus, FunctionReport};

/// Priority change below this is treated as noise.
pub const PRIORITY_EPSILON: f64 = 0.5;

/// MI drop threshold used by the secondary "worsened" rule.
pub const MI_DROP_THRESHOLD: f64 = 50.0;

/// Aggregate delta outcome counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeltaStats {
    pub new: usize,
    pub worsened: usize,
    pub improved: usize,
    pub unchanged: usize,
}

/// Match two paths even when one run used a different root prefix
/// (`crates/halmi-core/src/lib.rs` vs `halmi-core/src/lib.rs`).
#[must_use]
pub fn paths_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let norm = |p: &str| p.replace('\\', "/");
    let a = norm(a);
    let b = norm(b);
    let suffix = |long: &str, short: &str| {
        long.ends_with(short)
            && (long.len() == short.len() || long.as_bytes()[long.len() - short.len() - 1] == b'/')
    };
    suffix(&a, &b) || suffix(&b, &a)
}

/// Attach `delta` / `delta_status` to every current report by matching the
/// baseline on (path, name). Functions without a baseline match get status
/// `"new"`.
pub fn attach_delta(current: &mut [FunctionReport], baseline: &[FunctionReport]) -> DeltaStats {
    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, b) in baseline.iter().enumerate() {
        by_name.entry(b.name.as_str()).or_default().push(i);
    }
    let mut stats = DeltaStats::default();
    for r in current.iter_mut() {
        let matched = by_name.get(r.name.as_str()).and_then(|idxs| {
            idxs.iter()
                .position(|&i| paths_match(&baseline[i].path, &r.path))
                .map(|pos| idxs[pos])
        });

        if let Some(i) = matched {
            if let Some(idxs) = by_name.get_mut(r.name.as_str()) {
                idxs.retain(|&x| x != i);
            }
            r.mark_delta(Some(&baseline[i]));
        } else {
            r.mark_delta(None);
        }
        match r.delta_status {
            Some(DeltaStatus::Worsened) => stats.worsened += 1,
            Some(DeltaStatus::Improved) => stats.improved += 1,
            Some(DeltaStatus::New) => stats.new += 1,
            _ => stats.unchanged += 1,
        }
    }
    stats
}

impl FunctionReport {
    /// Compare against the baseline entry for the same function and store the
    /// per-field deltas with their verdict; `None` marks the function as new.
    pub(crate) fn mark_delta(&mut self, baseline: Option<&FunctionReport>) {
        let Some(b) = baseline else {
            self.delta = None;
            self.delta_status = Some(DeltaStatus::New);
            return;
        };
        let d = DeltaInfo {
            priority: self.priority - b.priority,
            mi_vs: self.mi_vs - b.mi_vs,
            effort: self.effort - b.effort,
        };
        self.delta_status = Some(
            if d.priority > PRIORITY_EPSILON
                || (self.mi_vs < MI_DROP_THRESHOLD && d.mi_vs < 0.0 && d.effort > 0.0)
            {
                DeltaStatus::Worsened
            } else if d.priority < -PRIORITY_EPSILON {
                DeltaStatus::Improved
            } else {
                DeltaStatus::Unchanged
            },
        );
        self.delta = Some(d);
    }
}

/// Keep only functions that got worse (for `--changed-only`).
pub fn retain_worsened(reports: &mut Vec<FunctionReport>) {
    reports.retain(|r| r.delta_status == Some(DeltaStatus::Worsened));
}
