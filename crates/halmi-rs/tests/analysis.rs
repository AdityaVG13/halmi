#![allow(
    clippy::suboptimal_flops,
    reason = "reference values restate the published a*b + c formulas on purpose; fused multiply-add would make the oracle round differently from the implementation"
)]
//! The Rust adapter contract: spans, names, Halstead counts, CC/LOC/nesting,
//! exclusions, error degradation, and the full pipeline with ranking.

mod common;

use common::{analyze, find};
use halmi_core::ranking::{PriorityConfig, finalize};

#[test]
fn simple_function_counts_exactly() {
    let src = "fn add(a: u32, b: u32) -> u32 {\n    a + b\n}\n";
    let reports = analyze(src);
    assert_eq!(reports.len(), 1);
    let r = &reports[0];
    assert_eq!(r.name, "add");
    assert_eq!((r.start_line, r.end_line), (1, 3));
    assert_eq!((r.volume, r.difficulty, r.effort), (8.0, 1.0, 8.0));
    assert_eq!(r.cyclomatic, 1);
    assert_eq!(r.loc, 1);
    assert_eq!(r.nesting, 0);
}

#[test]
fn fib_counts_exactly() {
    let src = concat!(
        "fn fib(n: u64) -> u64 {\n",
        "    if n < 2 {\n",
        "        n\n",
        "    } else {\n",
        "        fib(n - 1) + fib(n - 2)\n",
        "    }\n",
        "}\n",
    );
    let reports = analyze(src);
    let r = find(&reports, "fib");
    assert_eq!((r.difficulty, r.effort), (6.75, 381.191));
    assert!((r.volume - 56.473).abs() < 0.001, "{}", r.volume);
    assert_eq!(r.cyclomatic, 2);
    assert_eq!(r.loc, 3);
    assert_eq!(r.nesting, 1);
    assert_eq!((r.start_line, r.end_line), (1, 7));
}

#[test]
fn trivial_and_macro_only_bodies() {
    let reports = analyze("fn empty() {}\n");
    let empty = find(&reports, "empty");
    assert_eq!(empty.volume, 0.0);
    assert_eq!(empty.loc, 0);
    assert_eq!(empty.mi_vs, 100.0);
    assert_eq!(empty.mi_original, 100.0);

    let reports = analyze("fn log_it() {\n    println!(\"hi\");\n}\n");
    let log_it = find(&reports, "log_it");
    assert_eq!(log_it.volume, 0.0);
    assert_eq!(log_it.mi_vs, 100.0);
    assert_eq!(log_it.loc, 1);

    let mut reports = analyze("fn empty() {}\nfn log_it() {\n    println!(\"hi\");\n}\n");
    finalize(&mut reports, &PriorityConfig::default());
    assert!(reports.iter().all(|r| r.priority == 0.0));
    assert!(reports.iter().all(|r| r.reason == "trivial function"));
}

#[test]
fn control_flow_metrics_nesting_and_match_arms() {
    let deep = concat!(
        "fn deep(x: u32) {\n",
        "    for i in 0..x {\n",
        "        if i > 2 {\n",
        "            if x > 10 {\n",
        "                y();\n",
        "            }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let reports = analyze(deep);
    let r = find(&reports, "deep");
    assert_eq!(r.nesting, 3);
    assert_eq!(r.cyclomatic, 4);
    assert_eq!(r.loc, 4);
    assert_eq!(r.difficulty, 4.0);
    assert_eq!((r.start_line, r.end_line), (1, 9));

    let pick = concat!(
        "fn pick(x: u32) -> u32 {\n",
        "    match x {\n",
        "        0 => 1,\n",
        "        1 => 2,\n",
        "        _ => 3,\n",
        "    }\n",
        "}\n",
    );
    let reports = analyze(pick);
    let r = find(&reports, "pick");
    assert_eq!(r.cyclomatic, 4);
    assert_eq!(r.loc, 4);
    assert_eq!(r.nesting, 1);
}

#[test]
fn logical_operators_and_closures_raise_cc() {
    let src = concat!(
        "fn f(a: bool, b: bool) -> bool {\n",
        "    let c = |x: bool| x && a;\n",
        "    c(b) && b || a\n",
        "}\n",
    );
    let reports = analyze(src);
    let r = find(&reports, "f");
    assert_eq!(r.cyclomatic, 4);
    assert_eq!(r.loc, 2);
    assert_eq!(r.nesting, 0);
}

#[test]
fn method_chains_count_along_the_expression_spine() {
    let src = concat!(
        "fn chainy(v: Vec<u32>) -> u32 {\n",
        "    v.iter().map(|x| x + 1).filter(|x| *x > 2).take(3).sum()\n",
        "}\n",
    );
    let reports = analyze(src);
    let r = find(&reports, "chainy");
    assert_eq!(r.method_chain, 5);

    let src = "fn mixed(v: Vec<u32>) -> u32 {\n    v.iter().map(|x| x.max(3)).sum()\n}\n";
    let reports = analyze(src);
    let r = find(&reports, "mixed");
    assert_eq!(r.method_chain, 3, "{}", r.method_chain);
}

#[test]
fn try_cast_and_deref_classify_as_operators() {
    let src = "fn g(v: Vec<u8>) -> Option<u8> {\n    *v.get(0)? as u8\n}\n";
    let reports = analyze(src);
    let r = find(&reports, "g");
    assert_eq!(r.cyclomatic, 2);
    let ops = 5.0;
    let operands = 4.0;
    assert!(
        (r.volume - (ops + operands) * 9.0f64.log2()).abs() < 0.01,
        "{}",
        r.volume
    );
}

#[test]
fn qualified_names_and_precise_spans() {
    let src = concat!(
        "impl Foo {\n",
        "    fn bar(&self) -> u32 {\n",
        "        self.value\n",
        "    }\n",
        "}\n",
    );
    let reports = analyze(src);
    let r = find(&reports, "Foo::bar");
    assert_eq!((r.start_line, r.end_line), (2, 4));

    assert_eq!(r.difficulty, 1.0);
    assert_eq!(r.volume, 8.0);

    let trait_src = concat!(
        "trait T {\n",
        "    fn m(&self) -> u32 {\n",
        "        1\n",
        "    }\n",
        "    fn n(&self);\n",
        "}\n",
    );
    let reports = analyze(trait_src);
    assert_eq!(reports.len(), 1, "only default bodies are analyzed");
    assert_eq!(reports[0].name, "T::m");
    assert_eq!((reports[0].start_line, reports[0].end_line), (2, 4));

    let mod_src = concat!(
        "mod inner {\n",
        "    pub fn helper() -> u8 {\n",
        "        3\n",
        "    }\n",
        "}\n",
    );
    let reports = analyze(mod_src);
    let r = find(&reports, "helper");
    assert_eq!((r.start_line, r.end_line), (2, 4));
    assert_eq!(r.volume, 2.0);

    let marked = concat!(
        "/// doc line\n",
        "#[inline]\n",
        "pub fn marked() -> u32 {\n",
        "    7\n",
        "}\n",
    );
    let reports = analyze(marked);
    let r = find(&reports, "marked");
    assert_eq!((r.start_line, r.end_line), (3, 5));
}

#[test]
fn async_unsafe_spans_cover_the_whole_signature() {
    let src = concat!(
        "async fn h(x: &u8) -> u8 {\n",
        "    unsafe {\n",
        "        *x\n",
        "    }\n",
        "}\n",
        "fn caller(f: u8) -> u8 {\n",
        "    f\n",
        "}\n",
    );
    let reports = analyze(src);
    assert_eq!(reports.len(), 2);
    let h = find(&reports, "h");

    assert_eq!(h.volume, 8.0);
    assert_eq!(h.cyclomatic, 1);
    assert_eq!((h.start_line, h.end_line), (1, 5));
}

#[test]
fn nested_items_are_excluded_from_parent_counts() {
    let src = concat!(
        "fn outer() -> u32 {\n",
        "    fn inner() -> u32 {\n",
        "        9\n",
        "    }\n",
        "    inner()\n",
        "}\n",
    );
    let reports = analyze(src);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].name, "outer");
    assert_eq!(reports[0].loc, 1);
}

#[test]
fn comments_produce_sei_and_empty_comment_spans_do_not() {
    let src = "fn documented() {\n    // note\n    let x = 1; // trailing\n}\n";
    let reports = analyze(src);
    let r = find(&reports, "documented");

    let expected_sei = 158.020_453_692_617_05 + 50.0 * (1.2f64).sqrt().sin();
    assert!(
        (r.mi_sei.unwrap() - expected_sei).abs() < 0.01,
        "{}",
        r.mi_sei.unwrap()
    );
    assert!(r.mi_sei.unwrap() > r.mi_original);

    let src = "fn f() {\n    let x = 1;\n}\n";
    let reports = analyze(src);
    let r = find(&reports, "f");
    assert_eq!(r.mi_sei, Some(r.mi_original));
}

#[test]
fn parse_errors_degrade_to_structured_errors() {
    let bad = "fn broken( {\n";
    let a = halmi_rs::analyze_source("bad.rs", bad);
    assert_eq!(a.functions, [] as [halmi_core::FunctionReport; 0]);
    assert!(a.error.is_some());
    assert_eq!(a.path, "bad.rs");
}

#[test]
fn full_pipeline_ranks_and_analysis_is_idempotent() {
    let src = concat!(
        "fn small() -> u32 { 1 }\n",
        "fn big(a: u32, b: u32, c: u32) -> u32 {\n",
        "    let mut acc = a * b + c - a / b % c;\n",
        "    if acc > 10 {\n",
        "        acc += big(acc, b, c) * 2;\n",
        "    }\n",
        "    acc\n",
        "}\n",
    );
    let mut reports = analyze(src);
    let stats = finalize(&mut reports, &PriorityConfig::default());
    assert_eq!(stats.meaningful, 2);
    assert!(reports[0].priority > reports[1].priority);
    let big = find(&reports, "big");
    assert!(big.priority > 0.0);
    assert_ne!(big.reason, "");

    let mut again = analyze(src);
    finalize(&mut again, &PriorityConfig::default());
    assert_eq!(again.len(), reports.len());
    for (a, b) in again.iter().zip(reports.iter()) {
        assert_eq!(
            (
                a.name.as_str(),
                a.start_line,
                a.end_line,
                a.volume,
                a.effort
            ),
            (
                b.name.as_str(),
                b.start_line,
                b.end_line,
                b.volume,
                b.effort
            )
        );
    }
}

#[test]
fn self_counts_as_an_io_parameter() {
    let src = "impl Foo {\n    fn bar(&self, x: u32) -> u32 {\n        self.value + x\n    }\n}\n";
    let reports = analyze(src);
    let r = find(&reports, "Foo::bar");
    let expected = (2.0 + 3.0) * 5.0f64.log2();
    assert!(
        (r.potential_volume - expected).abs() < 1e-3,
        "{}",
        r.potential_volume
    );
}
