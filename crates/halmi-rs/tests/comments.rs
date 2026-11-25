//! The conservative comment scanner, exercised end-to-end through
//! `analyze_source`: the SEI comment term `50*sin(sqrt(2.4*cm))` exposes the
//! comment density of the function span, so each fixture's expected density
//! is hand-computed and asserted through that term.

mod common;

use common::analyze;

/// Assert the report's comment density matches `cm` via the SEI term.
fn assert_density(r: &halmi_core::report::FunctionReport, cm: f64) {
    let sei = r.mi_sei.expect("adapter always counts comments");
    let expected = 50.0 * (2.4 * cm).sqrt().sin();
    let actual = sei - r.mi_original;
    assert!(
        (actual - expected).abs() < 0.01,
        "{}: density term {actual} != {expected} (expected cm={cm})",
        r.name
    );
}

#[test]
fn line_block_and_nested_block_comments_count() {
    let src = "fn d() {\n// full\nlet x = 1; // trailing\n}\n";
    assert_density(&analyze(src)[0], 0.5);

    let src = "fn d() {\n/* a\nb */ let x = 1;\n}\n";
    assert_density(&analyze(src)[0], 0.5);

    let src = "fn d() {\n/* o /* i */ s */ let x = 1;\n}\n";
    assert_density(&analyze(src)[0], 1.0 / 3.0);
}

#[test]
fn byte_char_literals_do_not_desync_the_scanner() {
    let src = concat!(
        "fn f() {\n",
        "    let c = b'*';\n",
        "    let d = b'/';\n",
        "    let s = \"a/*b\";\n",
        "    let x = 1; // real\n",
        "}\n",
    );
    let reports = analyze(src);
    let r = &reports[0];
    assert_eq!(r.name, "f");
    let expected = 50.0 * (2.4f64 * (1.0 / 6.0)).sqrt().sin();
    assert!((r.mi_sei.unwrap() - r.mi_original - expected).abs() < 0.01);
}

#[test]
fn unterminated_block_comment_degrades_to_a_parse_error() {
    let src = "fn f() {\nlet x = 1;\n/* unterminated tail\n";
    let a = halmi_rs::analyze_source("bad.rs", src);
    assert_eq!(a.functions, [] as [halmi_core::FunctionReport; 0]);
    assert!(a.error.is_some());
}

#[test]
fn comment_markers_inside_literals_do_not_count() {
    let src = concat!(
        "fn d() {\n",
        "    let s = \"http://x // y\";\n",
        "    let r = r##\"say \"hi\" // no\"##;\n",
        "    let c = '/';\n",
        "}\n",
    );
    assert_density(&analyze(src)[0], 0.0);

    let src = "fn d<'a>(x: &'a str) {\nlet y = x;\n// c\n}\n";
    assert_density(&analyze(src)[0], 0.25);

    let src = concat!(
        "fn d() {\n",
        "    let s = \"line1\n",
        "line2 // inside string\";\n",
        "    // after\n",
        "}\n",
    );
    assert_density(&analyze(src)[0], 1.0 / 5.0);
}
