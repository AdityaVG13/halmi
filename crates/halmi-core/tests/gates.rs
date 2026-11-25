//! Joint gate semantics: violation only when ALL enabled gates fail.

mod common;

use common::tuned_report;
use halmi_core::gates::{Gates, violates_all, violations};

fn report(
    mi_vs: f64,
    difficulty: f64,
    effort: f64,
    loc: u32,
) -> halmi_core::report::FunctionReport {
    let mut r = tuned_report(effort, mi_vs, 1, loc);
    r.difficulty = difficulty;
    r.mi_band = halmi_core::metrics::MiBand::of(mi_vs);
    r
}

#[test]
fn min_band_gate_fails_when_band_is_worse_than_required() {
    let gates = Gates {
        min_band: Some(halmi_core::metrics::MiBand::High),
        ..Gates::default()
    };
    let high = report(90.0, 1.0, 50.0, 50);
    let mut low = report(90.0, 1.0, 50.0, 50);
    low.mi_vs = 30.0;
    low.mi_band = halmi_core::metrics::MiBand::of(30.0);
    assert!(!violates_all(&gates, &high, 100.0));
    assert!(violates_all(&gates, &low, 100.0));

    let gates = Gates {
        min_band: Some(halmi_core::metrics::MiBand::Moderate),
        ..Gates::default()
    };
    let mut moderate = report(90.0, 1.0, 50.0, 50);
    moderate.mi_vs = 70.0;
    moderate.mi_band = halmi_core::metrics::MiBand::of(70.0);
    assert!(!violates_all(&gates, &moderate, 100.0));
    assert!(violates_all(&gates, &low, 100.0));
}

#[test]
fn violation_requires_every_enabled_gate_to_fail() {
    let gates = Gates {
        min_mi: Some(40.0),
        max_difficulty: Some(30.0),
        max_effort_factor: None,
        min_band: None,
    };
    for (mi, diff, expect, why) in [
        (30.0, 10.0, false, "fails mi only"),
        (50.0, 45.0, false, "fails difficulty only"),
        (30.0, 45.0, true, "fails both"),
        (50.0, 10.0, false, "fails none"),
    ] {
        let r = report(mi, diff, 50.0, 50);
        assert_eq!(violates_all(&gates, &r, 100.0), expect, "{why}");
    }

    let r = report(0.0, 999.0, 999.0, 50);
    assert!(!violates_all(&Gates::default(), &r, 100.0));
    assert_eq!(
        violations(&Gates::default(), std::slice::from_ref(&r), 100.0),
        [] as [usize; 0]
    );
}

#[test]
fn effort_gate_scales_with_median_and_trivial_is_exempt() {
    let gates = Gates {
        max_effort_factor: Some(2.0),
        ..Gates::default()
    };
    assert!(!violates_all(&gates, &report(90.0, 1.0, 150.0, 50), 100.0));
    assert!(violates_all(&gates, &report(90.0, 1.0, 250.0, 50), 100.0));

    let strict = Gates {
        min_mi: Some(99.0),
        ..Gates::default()
    };
    assert!(!violates_all(&strict, &report(100.0, 0.0, 0.0, 0), 0.0));
}

#[test]
fn violations_reports_indices_in_order() {
    let gates = Gates {
        min_mi: Some(40.0),
        max_difficulty: Some(30.0),
        max_effort_factor: None,
        min_band: None,
    };
    let reports = vec![
        report(30.0, 45.0, 50.0, 50),
        report(50.0, 10.0, 50.0, 50),
        report(30.0, 45.0, 50.0, 50),
    ];
    assert_eq!(violations(&gates, &reports, 100.0), vec![0, 2]);
}
