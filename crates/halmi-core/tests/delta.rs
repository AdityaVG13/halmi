//! Baseline/delta mode: path matching, classification, and filtering.

mod common;

use common::tuned_report;
use halmi_core::delta::{DeltaStats, attach_delta, paths_match, retain_worsened};
use halmi_core::report::DeltaStatus;

fn report(
    name: &str,
    path: &str,
    priority: f64,
    mi_vs: f64,
    effort: f64,
) -> halmi_core::report::FunctionReport {
    let mut r = tuned_report(effort, mi_vs, 1, 30);
    r.path = path.into();
    r.name = name.into();
    r.priority = priority;
    r
}

#[test]
fn paths_match_across_root_prefixes_and_separators() {
    assert!(paths_match("crates/a/src/lib.rs", "a/src/lib.rs"));
    assert!(paths_match("a/src/lib.rs", "crates/a/src/lib.rs"));
    assert!(paths_match("src/lib.rs", "src/lib.rs"));
    assert!(paths_match("a\\src\\lib.rs", "src/lib.rs"));
    assert!(!paths_match("a/src/one.rs", "a/src/two.rs"));
    assert!(!paths_match("crates/a/src/lib.rs", "crates/b/src/lib.rs"));
}

#[test]
fn delta_classifies_worsened_improved_unchanged_new() {
    let mut current = vec![
        report("worse", "a.rs", 110.0, 40.0, 200.0),
        report("better", "a.rs", 90.0, 60.0, 100.0),
        report("same", "a.rs", 100.0, 60.0, 100.0),
        report("added", "a.rs", 500.0, 30.0, 900.0),
    ];
    let baseline = vec![
        report("worse", "a.rs", 100.0, 45.0, 150.0),
        report("better", "a.rs", 100.0, 55.0, 110.0),
        report("same", "a.rs", 100.0, 60.0, 100.0),
        report("removed", "a.rs", 80.0, 70.0, 50.0),
    ];
    let stats = attach_delta(&mut current, &baseline);
    assert_eq!(
        stats,
        DeltaStats {
            new: 1,
            worsened: 1,
            improved: 1,
            unchanged: 1
        }
    );
    assert_eq!(current[0].delta_status, Some(DeltaStatus::Worsened));
    assert_eq!(current[1].delta_status, Some(DeltaStatus::Improved));
    assert_eq!(current[2].delta_status, Some(DeltaStatus::Unchanged));
    assert_eq!(current[3].delta_status, Some(DeltaStatus::New));
    assert!(current[3].delta.is_none());
    let d = current[0].delta.unwrap();
    assert_eq!(d.priority, 10.0);

    let mut current = vec![report("f", "a.rs", 100.2, 45.0, 300.0)];
    let baseline = vec![report("f", "a.rs", 100.0, 60.0, 200.0)];
    assert_eq!(attach_delta(&mut current, &baseline).worsened, 1);
}

#[test]
fn duplicate_names_pair_one_to_one() {
    let mut current = vec![
        report("new", "a.rs", 100.0, 60.0, 100.0),
        report("new", "b.rs", 100.0, 60.0, 100.0),
    ];
    let baseline = vec![
        report("new", "a.rs", 100.0, 60.0, 100.0),
        report("new", "b.rs", 100.0, 60.0, 100.0),
    ];
    let stats = attach_delta(&mut current, &baseline);
    assert_eq!(
        stats,
        DeltaStats {
            new: 0,
            worsened: 0,
            improved: 0,
            unchanged: 2
        }
    );
}

#[test]
fn retain_worsened_keeps_only_worsened() {
    let mut current = vec![
        report("w", "a.rs", 110.0, 40.0, 200.0),
        report("u", "a.rs", 100.0, 60.0, 100.0),
        report("n", "a.rs", 100.0, 60.0, 100.0),
    ];
    let baseline = vec![
        report("w", "a.rs", 100.0, 45.0, 150.0),
        report("u", "a.rs", 100.0, 60.0, 100.0),
    ];
    attach_delta(&mut current, &baseline);
    retain_worsened(&mut current);
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].name, "w");
}
