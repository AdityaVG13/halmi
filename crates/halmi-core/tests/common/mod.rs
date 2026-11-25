//! Shared fixtures for the halmi-core integration tests.

use halmi_core::metrics::HalsteadCounts;
use halmi_core::report::{FunctionReport, RawFunction};

/// A non-trivial [`RawFunction`] with the given logical LOC and nesting.
/// Counts are fixed so volume/effort are always meaningful. (Not every test
/// target in this crate uses it.)
#[allow(dead_code)]
pub fn raw(loc: u32, nesting: u32) -> RawFunction {
    RawFunction {
        path: "a.rs".into(),
        language: halmi_core::report::Language::Rust,
        name: "f".into(),
        start_line: 1,
        end_line: loc + 1,
        counts: HalsteadCounts {
            eta1: 4,
            eta2: 4,
            n1: 20,
            n2: 20,
            io_params: 2,
        },
        cyclomatic: 5,
        loc,
        nesting,
        comment_density: None,
        method_chain: 0,
    }
}

/// A report whose ranking inputs (effort, `mi_vs`) are pinned directly so the
/// priority math can be asserted without re-deriving Halstead values. (Not
/// every test target in this crate uses it.)
#[allow(dead_code)]
pub fn tuned_report(effort: f64, mi_vs: f64, nesting: u32, loc: u32) -> FunctionReport {
    let mut r = FunctionReport::from_raw(&raw(loc, nesting));
    r.effort = effort;
    r.mi_vs = mi_vs;
    r
}

/// Floating-point equality at the tolerance formulas.rs hand values use.
/// (Not every test target in this crate uses it.)
#[allow(dead_code)]
pub fn assert_close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}
