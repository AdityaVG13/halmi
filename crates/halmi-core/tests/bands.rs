//! MI banding per docs/internal/halmi-formulas.md: High >= 85,
//! Moderate 65-85, Low < 65 on the `mi_vs` (0-100) scale.

mod common;

use common::raw;
use halmi_core::metrics::MiBand;
use halmi_core::report::FunctionReport;

#[test]
fn band_boundaries_follow_the_documented_thresholds() {
    assert_eq!(MiBand::of(100.0), MiBand::High);
    assert_eq!(MiBand::of(85.0), MiBand::High);
    assert_eq!(MiBand::of(84.999), MiBand::Moderate);
    assert_eq!(MiBand::of(65.0), MiBand::Moderate);
    assert_eq!(MiBand::of(64.999), MiBand::Low);
    assert_eq!(MiBand::of(0.0), MiBand::Low);
    assert_eq!(MiBand::High.as_str(), "high");
    assert_eq!(MiBand::Moderate.as_str(), "moderate");
    assert_eq!(MiBand::Low.as_str(), "low");
}

#[test]
fn reports_carry_their_band() {
    let r = FunctionReport::from_raw(&raw(10, 2));
    assert_eq!(r.mi_band, MiBand::of(r.mi_vs));

    let mut trivial = raw(0, 0);
    trivial.counts = halmi_core::metrics::HalsteadCounts {
        eta1: 1,
        eta2: 0,
        n1: 1,
        n2: 0,
        io_params: 0,
    };
    let t = FunctionReport::from_raw(&trivial);
    assert_eq!(t.mi_vs, 100.0);
    assert_eq!(t.mi_band, MiBand::High);
}

#[test]
fn band_matches_the_rounded_wire_value() {
    for (eta1, eta2, n1, n2, cc, loc) in [
        (4, 3, 7, 6, 3, 10),
        (5, 5, 30, 30, 5, 30),
        (2, 2, 3, 3, 1, 1),
    ] {
        let mut raw = raw(loc, 1);
        raw.counts = halmi_core::metrics::HalsteadCounts {
            eta1,
            eta2,
            n1,
            n2,
            io_params: 2,
        };
        raw.cyclomatic = cc;
        let r = halmi_core::report::FunctionReport::from_raw(&raw);
        assert_eq!(
            r.mi_band,
            halmi_core::metrics::MiBand::of(r.mi_vs),
            "mi_vs {}",
            r.mi_vs
        );
    }
}

#[test]
fn paths_match_requires_a_segment_boundary() {
    use halmi_core::delta::paths_match;
    assert!(paths_match("crates/a/src/lib.rs", "a/src/lib.rs"));
    assert!(!paths_match("xsrc/lib.rs", "src/lib.rs"));
    assert!(!paths_match("src/lib.rs", "xsrc/lib.rs"));
}
