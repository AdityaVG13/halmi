#![allow(
    clippy::suboptimal_flops,
    reason = "reference values restate the published a*b + c formulas on purpose; fused multiply-add would make the oracle round differently from the implementation"
)]
use halmi_core::adapter::LanguageAdapter;
use halmi_core::ranking::{PriorityConfig, finalize};
use halmi_python::PythonAdapter;

fn find<'a>(
    reports: &'a [halmi_core::report::FunctionReport],
    name: &str,
) -> &'a halmi_core::report::FunctionReport {
    reports
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("{name} not found in {reports:?}"))
}

const SRC: &str = "\
def add(a, b):
    return a + b

class Calc:
    def pick(self, x):
        # decide
        if x > 1:
            return x * 2
        return 0

def chained(v):
    return v.strip().lower().split()
";

#[test]
fn spans_names_and_counts() {
    let a = PythonAdapter.analyze("t.py", SRC);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(
        a.functions.len(),
        3,
        "{:?}",
        a.functions
            .iter()
            .map(|f| f.name.clone())
            .collect::<Vec<_>>()
    );

    let add = find(&a.functions, "add");
    assert_eq!((add.start_line, add.end_line), (1, 2));
    assert_eq!(add.io_field(), (2.0 + 2.0) * 4.0f64.log2());

    let pick = find(&a.functions, "Calc::pick");
    assert_eq!((pick.start_line, pick.end_line), (5, 9));
    assert_eq!(pick.cyclomatic, 2);
    assert_eq!(pick.loc, 3);
    assert_eq!(pick.loc, 3);

    let chained = find(&a.functions, "chained");
    assert_eq!(chained.method_chain, 3, "{:?}", chained.method_chain);
}

trait IoField {
    fn io_field(&self) -> f64;
}
impl IoField for halmi_core::report::FunctionReport {
    fn io_field(&self) -> f64 {
        self.potential_volume
    }
}

#[test]
fn shared_pipeline_and_determinism() {
    let mut functions = PythonAdapter.analyze("t.py", SRC).functions;
    let stats = finalize(&mut functions, &PriorityConfig::default());
    assert!(stats.meaningful >= 2);
    assert!(functions.iter().all(|r| !r.reason.is_empty()));
    let mut again = PythonAdapter.analyze("t.py", SRC).functions;
    finalize(&mut again, &PriorityConfig::default());
    let names: Vec<_> = functions.iter().map(|r| r.name.as_str()).collect();
    let names2: Vec<_> = again.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, names2);
}

#[test]
fn literals_do_not_break_comments_or_strings() {
    let src = "def f(s):\n    a = '# not a comment'\n    b = f'{x} # no'\n    c = \"\"\"multi\n#line\"\"\"\n    # real comment\n    return a + b + c\n";
    let a = PythonAdapter.analyze("t.py", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let f = find(&a.functions, "f");
    assert_eq!(f.loc, 4);
    let expected = f.mi_original + 50.0 * (2.4f64 * (1.0 / 7.0)).sqrt().sin();
    assert!((f.mi_sei.unwrap() - expected).abs() < 0.02);
}

#[test]
fn continuations_and_brackets_join_lines() {
    let src = "def f(x):\n    total = (x + 1 +\n             x * 2)\n    return total\n";
    let a = PythonAdapter.analyze("t.py", src);
    assert_eq!(a.error, None);
    let f = find(&a.functions, "f");
    assert_eq!(f.loc, 2, "bracket continuation joins logical lines");
    assert_eq!((f.start_line, f.end_line), (1, 4));
}

#[test]
fn nested_defs_excluded_from_parent() {
    let src = "def outer(x):\n    def inner(y):\n        return y * 3\n    return inner(x)\n";
    let a = PythonAdapter.analyze("t.py", src);
    assert_eq!(a.functions.len(), 1, "nested def not reported");
    assert_eq!(a.functions[0].name, "outer");
}

#[test]
fn soft_keywords_are_valid_identifiers() {
    let src = "def case(x):\n    m = x.match\n    return m\n";
    let a = PythonAdapter.analyze("t.py", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(a.functions.len(), 1);
    assert_eq!(a.functions[0].name, "case");
}

#[test]
fn unicode_source_never_panics_and_lexes() {
    let src = "def widen(x: int) -> int:\n    arrow = x \u{2192} 1\n    return arrow\n";
    let a = PythonAdapter.analyze("uni.py", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(a.functions.len(), 1);
}

#[test]
fn degraded_files_never_panic() {
    for bad in [
        "def broken(:\n    pass\n",
        "x = 'never closed\n",
        "def f():\n        return 1\n     return 2\n",
    ] {
        let a = PythonAdapter.analyze("bad.py", bad);
        let _ = &a.functions;
    }
}

#[test]
fn annotations_count_as_output() {
    let src = "def typed(x: int) -> int:\n    return x\n";
    let a = PythonAdapter.analyze("t.py", src);
    let f = find(&a.functions, "typed");
    assert_eq!(f.potential_volume, (2.0 + 2.0) * 4.0f64.log2());
}

#[test]
fn string_prefixes_are_part_of_the_literal() {
    let src =
        "def f():\n    a = f\"x {y}\"\n    b = r\"raw\\\\\"\n    c = b\"bytes\"\n    return a\n";
    let a = PythonAdapter.analyze("t.py", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let f = find(&a.functions, "f");
    assert!(f.volume > 0.0);
    let raw = "def g():\n    s = r\"\\\\\"\n    return s\n";
    let a = PythonAdapter.analyze("t.py", raw);
    assert_eq!(a.error, None, "{:?}", a.error);
}

#[test]
fn separators_are_not_operators() {
    let src = "def f(a, b):\n    d = {1: 2}\n    return a + b\n";
    let a = PythonAdapter.analyze("t.py", src);
    let f = find(&a.functions, "f");
    assert!(
        f.volume < 30.0,
        "volume {} suggests separators counted",
        f.volume
    );
}

#[test]
fn nesting_tracks_indentation() {
    let src = "def f(x):\n    if x:\n        for i in x:\n            if i:\n                return i\n    return 0\n";
    let a = PythonAdapter.analyze("t.py", src);
    let f = find(&a.functions, "f");
    assert_eq!(f.nesting, 4, "indentation depth: body=1, if=2, for=3, if=4");
}

#[test]
fn every_method_in_a_class_is_qualified() {
    let src =
        "class C:\n    def one(self):\n        return 1\n\n    def two(self):\n        return 2\n";
    let a = PythonAdapter.analyze("t.py", src);
    let names: Vec<&str> = a.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"C::one"), "{names:?}");
    assert!(names.contains(&"C::two"), "{names:?}");
}

#[test]
fn lambda_does_not_truncate_the_parent_span() {
    let src = "def f(items):\n    g = lambda x: x + 1\n    return [g(i) for i in items]\n";
    let a = PythonAdapter.analyze("t.py", src);
    let f = find(&a.functions, "f");
    assert_eq!((f.start_line, f.end_line), (1, 3));
}

#[test]
fn none_is_an_operand() {
    let src = "def f(v):\n    if v is None:\n        return 0\n    return v\n";
    let a = PythonAdapter.analyze("t.py", src);
    let f = find(&a.functions, "f");
    assert!(f.volume > 0.0);
    let with_none = "def g():\n    return None\n";
    let a = PythonAdapter.analyze("t.py", with_none);
    let g = find(&a.functions, "g");
    assert!(g.volume > 0.0, "None should be an operand");
}

#[test]
fn one_line_suites_and_sequential_defs_keep_their_boundaries() {
    let src = "def oneline(): return 1\n\ndef nested():\n    if True:\n        return 1\ndef second():\n    return 2\n";
    let a = PythonAdapter.analyze("suites.py", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let names: Vec<&str> = a.functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["oneline", "nested", "second"]);

    let oneline = find(&a.functions, "oneline");
    assert_eq!((oneline.start_line, oneline.end_line), (1, 1));
    assert!(oneline.volume > 0.0);

    let nested = find(&a.functions, "nested");
    assert_eq!((nested.start_line, nested.end_line), (3, 5));
    assert_eq!(nested.cyclomatic, 2);
    assert_eq!(nested.nesting, 2);

    let second = find(&a.functions, "second");
    assert_eq!((second.start_line, second.end_line), (6, 7));
    assert!(second.volume > 0.0);
    assert_eq!(second.nesting, 1);
}

#[test]
fn scientific_notation_with_sign_is_one_number() {
    let src = "def f():\n    return 1e-5 + 2E+3\n";
    let a = PythonAdapter.analyze("t.py", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let f = find(&a.functions, "f");
    assert_eq!(f.loc, 1);
}

#[test]
fn raw_strings_with_escaped_quotes_lex() {
    let src =
        "def f():\n    a = r\"quote \\\" inside\"\n    b = r'apos \\' inside'\n    return a + b\n";
    let a = PythonAdapter.analyze("t.py", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(a.functions.len(), 1);
}

#[test]
fn declaration_and_separator_volume_is_exact() {
    let src = "def f(a, b):\n    return a + b\n";
    let a = PythonAdapter.analyze("t.py", src);
    let f = find(&a.functions, "f");
    assert!(
        (f.volume - 5.0 * 5.0f64.log2()).abs() < 1e-3,
        "{}",
        f.volume
    );
}
