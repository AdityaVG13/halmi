//! The shared metrics collector and C-family toolkit contracts.

use halmi_lex::cfamily;
use halmi_lex::{LangSpec, LocMode, MetricsCollector, Tok, density};

const RUSTISH: LangSpec = LangSpec {
    decl_op: "fn",
    keyword_operators: &["fn", "if", "else", "for", "while", "return"],
    cc_keywords: &["if", "for", "while"],
    separator_ops: halmi_lex::PAREN_SEPARATORS,
    cc_ops: &["&&", "||"],
    loc_mode: LocMode::Logical,
};

fn collect(spec: &LangSpec, toks: &[(&Tok, usize)]) -> halmi_lex::MetricsOutcome {
    let mut c = MetricsCollector::new(spec, 0);
    for (t, l) in toks {
        c.token(t, *l);
    }
    c.finish(0)
}

#[test]
fn decl_op_is_counted_once() {
    let toks = [(&Tok::Keyword("return"), 1), (&Tok::Ident("x"), 1)];
    let out = collect(&RUSTISH, &toks);
    assert_eq!(out.counts.eta1, 2, "fn + return");
    assert_eq!(out.counts.n1, 2);
    assert_eq!(out.counts.eta2, 1);
    assert_eq!(out.cyclomatic, 1);
}

#[test]
fn separators_are_not_operators() {
    let toks = [
        (&Tok::Keyword("return"), 1),
        (&Tok::Punct("("), 1),
        (&Tok::Ident("a"), 1),
        (&Tok::Punct(","), 1),
        (&Tok::Ident("b"), 1),
        (&Tok::Punct(")"), 1),
    ];
    let out = collect(&RUSTISH, &toks);
    assert_eq!(out.counts.eta1, 2, "fn + return only");
}

#[test]
fn attr_is_an_operator() {
    let toks = [
        (&Tok::Ident("a"), 1),
        (&Tok::Punct("."), 1),
        (&Tok::Ident("b"), 1),
    ];
    let out = collect(&RUSTISH, &toks);
    assert_eq!(out.counts.eta1, 2, "fn + attr");
}

#[test]
fn cc_counts_keywords_and_ops() {
    let toks = [
        (&Tok::Keyword("if"), 1),
        (&Tok::Ident("a"), 1),
        (&Tok::Punct("&&"), 1),
        (&Tok::Ident("b"), 1),
        (&Tok::Keyword("for"), 1),
    ];
    let out = collect(&RUSTISH, &toks);
    assert_eq!(out.cyclomatic, 4, "1 + if + && + for");
}

#[test]
fn nesting_via_open_close() {
    let mut c = MetricsCollector::new(&RUSTISH, 0);
    c.open_block(1);
    c.open_block(1);
    c.close_block(1);
    c.open_block(1);
    c.close_block(1);
    c.close_block(1);
    let out = c.finish(0);
    assert_eq!(out.nesting, 2);
}

#[test]
fn nesting_via_set_depth() {
    let mut c = MetricsCollector::new(&RUSTISH, 1);
    c.set_depth(2);
    c.set_depth(3);
    c.set_depth(2);
    c.set_depth(1);
    let out = c.finish(0);
    assert_eq!(out.nesting, 3);
}

#[test]
fn reset_restores_the_constructor_base_depth() {
    let mut c = MetricsCollector::new(&RUSTISH, 1);
    c.set_depth(3);
    c.set_depth(2);
    assert_eq!(c.finish(0).nesting, 3);
    assert_eq!(c.current_depth(), 1);
    assert_eq!(c.finish(0).nesting, 1);
}

#[test]
fn method_chain_semantics() {
    let mut c = MetricsCollector::new(&RUSTISH, 0);
    c.call(true, false);
    c.call(true, true);
    c.call(false, false);
    c.call(true, true);
    let out = c.finish(0);
    assert_eq!(
        out.method_chain, 2,
        "fresh=1, extends=2, plain resets, extends-after-reset=1"
    );
}

#[test]
fn loc_logical_counts_newline_delimited_lines() {
    let spec = LangSpec {
        loc_mode: LocMode::Logical,
        ..RUSTISH
    };
    let toks = [
        (&Tok::Keyword("return"), 1),
        (&Tok::Ident("x"), 1),
        (&Tok::Newline, 1),
        (&Tok::Keyword("return"), 2),
        (&Tok::Ident("y"), 2),
        (&Tok::Newline, 2),
    ];
    let out = collect(&spec, &toks);
    assert_eq!(out.loc, 2);
}

#[test]
fn loc_physical_counts_distinct_lines() {
    let spec = LangSpec {
        loc_mode: LocMode::Physical,
        ..RUSTISH
    };
    let toks = [
        (&Tok::Keyword("return"), 1),
        (&Tok::Ident("a"), 1),
        (&Tok::Ident("b"), 2),
        (&Tok::Newline, 2),
    ];
    let out = collect(&spec, &toks);
    assert_eq!(out.loc, 2);
}

#[test]
fn density_matches_span_fraction() {
    let map = vec![false, true, true, false, true];
    assert!((density(&map, 2, 4) - 2.0 / 3.0).abs() < 1e-9);
    assert_eq!(density(&map, 5, 2), 0.0);
}

#[test]
fn cfamily_punct_longest_match() {
    assert_eq!(
        cfamily::punct_len("a => b", 2),
        2,
        "arrow must lex as one token"
    );
    assert_eq!(cfamily::punct_len("a == b", 2), 2);
    assert_eq!(cfamily::punct_len("a >>>= b", 2), 4);
    assert_eq!(cfamily::punct_len("a ... b", 2), 3);
    assert_eq!(cfamily::punct_len("a == b", 2), 2);
    assert_eq!(cfamily::punct_len("a = b", 2), 1);
    assert_eq!(cfamily::punct_len("a b", 2), 0);
}

#[test]
fn cfamily_number_scanning() {
    for (src, want) in [
        ("0xFF", 4),
        ("0b1010", 6),
        ("123", 3),
        ("1.5", 3),
        ("1e-5", 4),
        ("1_000", 5),
        ("1..2", 1),
        ("1n", 2),
    ] {
        let end = cfamily::scan_number(src.as_bytes(), 0);
        assert_eq!(end, want, "{src}");
    }
}

#[test]
fn cfamily_string_and_raw_string() {
    let end = cfamily::scan_string(b"\"a\\\"b\" x", 0, 1).unwrap();
    assert_eq!(end, 6);
    assert!(cfamily::scan_string(b"\"abc", 0, 1).is_err());
    let mut line = 1;
    let end = cfamily::scan_raw_string(b"`a\nb`", 0, &mut line).unwrap();
    assert_eq!(end, 5);
    assert_eq!(line, 2);
}
