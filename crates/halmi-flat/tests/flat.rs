#![allow(
    clippy::suboptimal_flops,
    reason = "reference values restate the published a*b + c formulas on purpose; fused multiply-add would make the oracle round differently from the implementation"
)]
//! The adapter-port proof: flat-script flows through the exact shared
//! pipeline the Rust adapter uses (metrics, ranking, degradation contract).

use halmi_core::adapter::LanguageAdapter;
use halmi_core::ranking::{PriorityConfig, finalize};
use halmi_flat::FlatAdapter;

const SRC: &str = "\
def add:
    a + b - c
def decide:
    # note
    if a < b
    x * 2
def empty:
";

#[test]
fn flat_script_functions_carry_exact_spans_and_counts() {
    let a = FlatAdapter.analyze("t.flat", SRC);
    assert_eq!(a.error, None);
    assert_eq!(a.functions.len(), 3);

    let add = &a.functions[0];
    assert_eq!(add.name, "add");
    assert_eq!((add.start_line, add.end_line), (1, 2));

    assert_eq!(add.difficulty, (2.0 / 2.0) * (3.0 / 3.0));
    assert_eq!(add.loc, 1);
    assert_eq!(add.cyclomatic, 1);
    assert_eq!(add.method_chain, 0);
    assert_eq!(add.path, "t.flat");

    let decide = &a.functions[1];
    assert_eq!((decide.start_line, decide.end_line), (3, 6));
    assert_eq!(decide.cyclomatic, 2);
    assert_eq!(decide.loc, 2);

    let empty = &a.functions[2];
    assert_eq!((empty.start_line, empty.end_line), (7, 7));
    assert_eq!(empty.loc, 0);
}

#[test]
fn flat_script_ranks_through_the_shared_pipeline() {
    let mut functions = FlatAdapter.analyze("t.flat", SRC).functions;
    let stats = finalize(&mut functions, &PriorityConfig::default());
    assert!(stats.meaningful >= 2);
    assert!(functions.iter().all(|r| !r.reason.is_empty()));

    let mut again = FlatAdapter.analyze("t.flat", SRC).functions;
    finalize(&mut again, &PriorityConfig::default());
    let names: Vec<_> = functions.iter().map(|r| r.name.as_str()).collect();
    let names2: Vec<_> = again.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, names2);
}

#[test]
fn comment_density_counts_hash_lines_in_the_span() {
    let a = FlatAdapter.analyze("t.flat", SRC);
    let decide = &a.functions[1];
    let expected = decide.mi_original + 50.0 * (2.4 * 0.25f64).sqrt().sin();
    assert!((decide.mi_sei.unwrap() - expected).abs() < 0.01);
}

#[test]
fn malformed_structure_degrades_to_error_never_panics() {
    let a = FlatAdapter.analyze("bad.flat", "def\n    oops");
    assert_eq!(a.functions, [] as [halmi_core::FunctionReport; 0]);
    assert!(a.error.is_some());
    let b = FlatAdapter.analyze("bad.flat", "stray top-level line");
    assert_eq!(b.functions, [] as [halmi_core::FunctionReport; 0]);
    assert!(b.error.is_some());
}
