#![allow(
    clippy::suboptimal_flops,
    reason = "reference values restate the published a*b + c formulas on purpose; fused multiply-add would make the oracle round differently from the implementation"
)]
//! Joint priority scoring, reason strings, and repository ranking order.

mod common;

use common::assert_close;
use common::tuned_report;
use halmi_core::ranking::{PriorityConfig, compute_priority, finalize, reason_for};

#[test]
fn priority_matches_documented_formula() {
    let cfg = PriorityConfig::default();
    let p = compute_priority(&cfg, 200.0, 40.0, 4, 100, 100.0);
    let expected = 2.0 * 60.0 * 1.3 * (1.0 + 0.2 * 100.0f64.ln());
    assert!((p - expected).abs() < 1e-9);
    assert!((p - 299.681_309_802_828_5).abs() < 1e-9);

    let p = compute_priority(&cfg, 50.0, 50.0, 1, 10, 0.0);
    assert!((p - 50.0 * 50.0 * 1.0 * (1.0 + 0.2 * 10.0f64.ln())).abs() < 1e-9);
}

#[test]
fn finalize_ranks_desc_and_flags_trivial() {
    let mut reports = vec![
        tuned_report(0.0, 100.0, 0, 0),
        tuned_report(1000.0, 30.0, 5, 200),
    ];
    let stats = finalize(&mut reports, &PriorityConfig::default());
    assert_eq!(stats.meaningful, 1);
    assert_eq!(reports[0].name, "f");
    assert!(reports[0].priority > 0.0);
    assert_eq!(reports[1].priority, 0.0);
    assert_eq!(reports[1].reason, "trivial function");
    assert_close(stats.median_effort, 1000.0);
}

#[test]
fn finalize_tiebreaks_and_is_deterministic() {
    let mut reports = vec![
        tuned_report(10.0, 90.0, 0, 5),
        tuned_report(500.0, 20.0, 3, 200),
        tuned_report(500.0, 20.0, 3, 200),
    ];
    reports[1].name = "b".into();
    reports[2].name = "a".into();
    finalize(&mut reports, &PriorityConfig::default());
    let names: Vec<&str> = reports.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["a", "b", "f"],
        "name tiebreak then trivial last"
    );
    assert_eq!(reports[0].priority, reports[1].priority);
    assert!(reports[2].priority < reports[1].priority);

    let mut clone = vec![
        tuned_report(10.0, 90.0, 0, 5),
        tuned_report(500.0, 20.0, 3, 200),
        tuned_report(500.0, 20.0, 3, 200),
    ];
    clone[1].name = "b".into();
    clone[2].name = "a".into();
    finalize(&mut clone, &PriorityConfig::default());
    for (a, b) in reports.iter().zip(clone.iter()) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.priority.to_bits(), b.priority.to_bits());
    }
    finalize(&mut reports, &PriorityConfig::default());
    let names: Vec<&str> = reports.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["a", "b", "f"]);
}

/// One signal combination fed to `reason_for`, with the clause it must name.
struct ReasonCase {
    name: &'static str,
    effort: f64,
    mi_vs: f64,
    difficulty: f64,
    cyclomatic: u32,
    purity: f64,
    bugs: f64,
    nesting: u32,
    loc: u32,
    chain: u32,
    expect: &'static str,
}

/// Each case isolates one ranking signal; the expected clause is the
/// signal's documented phrase.
const REASON_CASES: &[ReasonCase] = &[
    ReasonCase {
        name: "dense-and-large",
        effort: 500.0,
        mi_vs: 30.0,
        difficulty: 1.0,
        cyclomatic: 5,
        purity: 0.9,
        bugs: 0.0,
        nesting: 1,
        loc: 100,
        chain: 0,
        expect: "dense and large",
    },
    ReasonCase {
        name: "dense-expressions",
        effort: 50.0,
        mi_vs: 60.0,
        difficulty: 45.0,
        cyclomatic: 3,
        purity: 0.9,
        bugs: 0.0,
        nesting: 0,
        loc: 20,
        chain: 0,
        expect: "dense expressions",
    },
    ReasonCase {
        name: "repeated-patterns",
        effort: 50.0,
        mi_vs: 80.0,
        difficulty: 1.0,
        cyclomatic: 5,
        purity: 0.5,
        bugs: 0.0,
        nesting: 0,
        loc: 40,
        chain: 0,
        expect: "repeated patterns",
    },
    ReasonCase {
        name: "defect-signal",
        effort: 50.0,
        mi_vs: 80.0,
        difficulty: 1.0,
        cyclomatic: 5,
        purity: 0.9,
        bugs: 2.0,
        nesting: 0,
        loc: 10,
        chain: 0,
        expect: "elevated defect signal",
    },
    ReasonCase {
        name: "deep-nesting",
        effort: 50.0,
        mi_vs: 80.0,
        difficulty: 1.0,
        cyclomatic: 5,
        purity: 0.9,
        bugs: 0.0,
        nesting: 5,
        loc: 10,
        chain: 0,
        expect: "deep nesting",
    },
    ReasonCase {
        name: "high-branching",
        effort: 50.0,
        mi_vs: 80.0,
        difficulty: 1.0,
        cyclomatic: 20,
        purity: 0.9,
        bugs: 0.0,
        nesting: 0,
        loc: 10,
        chain: 0,
        expect: "high branching",
    },
    ReasonCase {
        name: "low-mi-fallback",
        effort: 50.0,
        mi_vs: 60.0,
        difficulty: 1.0,
        cyclomatic: 5,
        purity: 0.9,
        bugs: 0.0,
        nesting: 0,
        loc: 10,
        chain: 0,
        expect: "low maintainability index",
    },
    ReasonCase {
        name: "no-signal",
        effort: 10.0,
        mi_vs: 90.0,
        difficulty: 1.0,
        cyclomatic: 5,
        purity: 0.9,
        bugs: 0.0,
        nesting: 0,
        loc: 5,
        chain: 0,
        expect: "no dominant signal",
    },
    ReasonCase {
        name: "method-chain",
        effort: 50.0,
        mi_vs: 60.0,
        difficulty: 45.0,
        cyclomatic: 3,
        purity: 0.9,
        bugs: 0.0,
        nesting: 0,
        loc: 20,
        chain: 5,
        expect: "long method chain",
    },
];

#[test]
fn reason_clauses_come_from_the_signal_combinations() {
    let median = 100.0;
    for c in REASON_CASES {
        let mut r = tuned_report(c.effort, c.mi_vs, c.nesting, c.loc);
        r.difficulty = c.difficulty;
        r.cyclomatic = c.cyclomatic;
        r.purity_ratio = c.purity;
        r.estimated_bugs = c.bugs;
        r.method_chain = c.chain;
        let reason = reason_for(&r, median);
        assert!(
            reason.contains(c.expect),
            "{name}: got {reason}",
            name = c.name
        );
    }

    let mut r = tuned_report(500.0, 30.0, 9, 300);
    r.purity_ratio = 0.5;
    r.estimated_bugs = 5.0;
    let reason = reason_for(&r, median);
    assert_eq!(reason.matches("; ").count(), 1, "{reason}");

    let trivial = tuned_report(0.0, 100.0, 0, 0);
    assert_eq!(reason_for(&trivial, median), "trivial function");
}
