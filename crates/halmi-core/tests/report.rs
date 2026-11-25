//! `FunctionReport` construction and JSON schema stability.

mod common;

use common::raw;
use halmi_core::report::FunctionReport;

#[test]
fn from_raw_computes_and_rounds_the_suite() {
    let mut raw = raw(10, 2);
    raw.counts = halmi_core::metrics::HalsteadCounts {
        eta1: 4,
        eta2: 3,
        n1: 7,
        n2: 6,
        io_params: 2,
    };
    raw.cyclomatic = 3;
    let r = FunctionReport::from_raw(&raw);
    assert_eq!(r.path, "a.rs");
    assert!((r.volume - 36.496).abs() < 1e-3);
    assert!((r.difficulty - 4.0).abs() < 1e-3);
    assert!((r.effort - 145.982).abs() < 1e-3);
    assert!((r.mi_vs - 66.844).abs() < 1e-3);
    assert!(r.mi_sei.is_none());

    assert_eq!(r.priority, 0.0);
    assert_eq!(r.reason, "");
    assert!(r.delta.is_none());
    assert!(r.delta_status.is_none());
}

#[test]
fn from_raw_trivial_maps_to_maintainability_100() {
    let mut trivial = raw(0, 0);
    trivial.counts = halmi_core::metrics::HalsteadCounts {
        eta1: 1,
        eta2: 0,
        n1: 1,
        n2: 0,
        io_params: 0,
    };
    trivial.path = "a.rs".into();
    let r = FunctionReport::from_raw(&trivial);
    assert_eq!(r.volume, 0.0);
    assert_eq!(r.mi_vs, 100.0);
    assert_eq!(r.mi_original, 100.0);
    assert_eq!(r.mi_sei, None);

    trivial.comment_density = Some(0.2);
    let r = FunctionReport::from_raw(&trivial);
    assert_eq!(r.mi_sei, Some(100.0));
}
