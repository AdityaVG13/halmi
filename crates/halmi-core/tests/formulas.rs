#![allow(
    clippy::suboptimal_flops,
    reason = "reference values restate the published a*b + c formulas on purpose; fused multiply-add would make the oracle round differently from the implementation"
)]
//! Halstead suite and MI variant formulas against hand-computed reference
//! values and the documented identities in docs/internal/halmi-formulas.md.

mod common;

use common::assert_close;
use halmi_core::metrics::{HalsteadCounts, MiInputs, compute_mi};

/// Exact `f64` value of one of the grid's small counts (at most 12).
#[expect(
    clippy::cast_precision_loss,
    reason = "grid counts are at most 12; every integer up to 2^53 converts exactly"
)]
const fn exact(n: usize) -> f64 {
    n as f64
}

const KNOWN: HalsteadCounts = HalsteadCounts {
    eta1: 4,
    eta2: 3,
    n1: 7,
    n2: 6,
    io_params: 0,
};

#[test]
fn halstead_known_values() {
    let m = KNOWN.compute();
    assert!(m.meaningful);
    assert_close(m.vocabulary, 7.0);
    assert_close(m.length, 13.0);
    assert_close(m.volume, 13.0 * 7.0f64.log2());
    assert_close(m.difficulty, 4.0);
    assert_close(m.effort, 4.0 * 13.0 * 7.0f64.log2());
    assert_close(m.level, 0.25);
    assert_close(m.time_seconds, m.effort / 18.0);
    assert_close(m.estimated_bugs, m.volume / 3000.0);
}

#[test]
fn potential_volume_uses_unique_io_params() {
    let mut counts = KNOWN;
    counts.io_params = 2;
    let m = counts.compute();
    assert_close(m.potential_volume, 8.0);

    counts.io_params = 0;
    let m = counts.compute();
    assert_close(m.potential_volume, 2.0);

    let mut counts = KNOWN;
    counts.io_params = 2;
    let m = counts.compute();
    assert!(m.volume >= m.potential_volume);
}

#[test]
fn halstead_fringe_known_values() {
    let m = KNOWN.compute();
    assert_close(m.estimated_length, 4.0 * 2.0 + 3.0 * 3.0f64.log2());
    assert_close(m.purity_ratio, m.estimated_length / 13.0);
    assert_close(m.intelligence, m.volume / m.difficulty);
    assert_close(m.language_level, 0.25 * 0.25 * m.volume);
    assert_close(m.alt_length_factorial, 7.169_925_001_442_312);
    assert_close(m.alt_length_cross, 12.339_850_002_884_624);
    assert_close(m.alt_length_root, 13.196_152_422_706_632);
}

#[test]
fn halstead_degenerate_counts_are_not_meaningful() {
    for counts in [
        HalsteadCounts::default(),
        HalsteadCounts {
            eta1: 2,
            eta2: 0,
            n1: 3,
            n2: 0,
            io_params: 0,
        },
        HalsteadCounts {
            eta1: 0,
            eta2: 3,
            n1: 0,
            n2: 9,
            io_params: 0,
        },
        HalsteadCounts {
            eta1: 1,
            eta2: 0,
            n1: 1,
            n2: 1,
            io_params: 0,
        },
    ] {
        let m = counts.compute();
        assert!(!m.meaningful, "{counts:?}");
        assert_close(m.volume, 0.0);
        assert_close(m.difficulty, 0.0);
        assert_close(m.purity_ratio, 0.0);
    }
}

#[test]
fn halstead_suite_identities_hold_across_count_grid() {
    for eta1 in 1..=6usize {
        for eta2 in 1..=6usize {
            for n1 in 1..=12usize {
                for n2 in 1..=12usize {
                    let m = HalsteadCounts {
                        eta1,
                        eta2,
                        n1,
                        n2,
                        io_params: 0,
                    }
                    .compute();
                    assert!(m.meaningful, "({eta1},{eta2},{n1},{n2})");
                    assert_close(m.vocabulary, exact(eta1 + eta2));
                    assert_close(m.length, exact(n1 + n2));
                    assert_close(m.effort, m.difficulty * m.volume);
                    assert_close(m.time_seconds, m.effort / 18.0);
                    assert_close(m.estimated_bugs, m.volume / 3000.0);
                    assert_close(m.level * m.difficulty, 1.0);
                    assert_close(m.intelligence, m.level * m.volume);
                    assert_close(m.language_level, m.level * m.level * m.volume);
                }
            }
        }
    }
}

#[test]
fn mi_variants_known_values() {
    let h = KNOWN.compute();
    let mi = compute_mi(
        &MiInputs {
            volume: h.volume,
            cyclomatic: 3,
            loc: 10,
            comment_density: Some(0.2),
        },
        h.meaningful,
    );
    assert!(mi.meaningful);
    let expected_original = 171.0 - 5.2 * h.volume.ln() - 0.23 * 3.0 - 16.2 * 10.0f64.ln();
    assert_close(mi.original, expected_original);
    assert_close(mi.vs, (expected_original * 100.0 / 171.0).max(0.0));
    assert_close(
        mi.sei.unwrap(),
        expected_original + 50.0 * (2.4 * 0.2f64).sqrt().sin(),
    );
    assert_close(mi.original, 114.302_722_632_664_78);
    assert_close(mi.vs, 66.843_697_446_002_8);
    assert_close(mi.sei.unwrap(), 146.238_213_165_515_08);
}

#[test]
fn mi_tiny_functions_map_to_100_and_vs_clamps_at_zero() {
    for (volume, loc) in [(0.0, 5u32), (1.0, 0), (0.0, 0)] {
        let mi = compute_mi(
            &MiInputs {
                volume,
                cyclomatic: 1,
                loc,
                comment_density: Some(0.5),
            },
            volume > 1.0,
        );
        assert!(!mi.meaningful, "volume={volume} loc={loc}");
        assert_close(mi.original, 100.0);
        assert_close(mi.vs, 100.0);
        assert_close(mi.sei.unwrap(), 100.0);
    }

    let mi = compute_mi(
        &MiInputs {
            volume: 1e9,
            cyclomatic: 400,
            loc: 10_000,
            comment_density: None,
        },
        true,
    );
    assert!(mi.original < 0.0);
    assert_close(mi.vs, 0.0);
}

#[test]
fn mi_sei_presence_and_comment_clamping() {
    let bare = compute_mi(
        &MiInputs {
            volume: 36.5,
            cyclomatic: 3,
            loc: 10,
            comment_density: None,
        },
        true,
    );
    assert!(bare.sei.is_none());
    assert!(bare.meaningful);

    let plain = compute_mi(
        &MiInputs {
            volume: 100.0,
            cyclomatic: 2,
            loc: 20,
            comment_density: None,
        },
        true,
    );
    let over = compute_mi(
        &MiInputs {
            volume: 100.0,
            cyclomatic: 2,
            loc: 20,
            comment_density: Some(5.0),
        },
        true,
    );
    let full = compute_mi(
        &MiInputs {
            volume: 100.0,
            cyclomatic: 2,
            loc: 20,
            comment_density: Some(1.0),
        },
        true,
    );
    assert_close(over.sei.unwrap(), full.sei.unwrap());
    assert!(over.sei.unwrap() > plain.original);

    let trivial_bare = compute_mi(
        &MiInputs {
            volume: 0.0,
            cyclomatic: 1,
            loc: 0,
            comment_density: None,
        },
        false,
    );
    assert!(trivial_bare.sei.is_none());
}
