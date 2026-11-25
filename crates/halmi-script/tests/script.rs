#![allow(
    clippy::suboptimal_flops,
    reason = "reference values restate the published a*b + c formulas on purpose; fused multiply-add would make the oracle round differently from the implementation"
)]
use halmi_core::adapter::LanguageAdapter;
use halmi_core::ranking::{PriorityConfig, finalize};
use halmi_script::ScriptAdapter;

fn find<'a>(
    reports: &'a [halmi_core::report::FunctionReport],
    name: &str,
) -> &'a halmi_core::report::FunctionReport {
    reports
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("{name} not found in {reports:?}"))
}

const JS: &str = "\
function add(a, b) {
  return a + b;
}

const twice = (x) => x * 2;

class Calc {
  pick(x) {
    // decide
    if (x > 1) {
      return x * 2;
    }
    return 0;
  }
}

const re = /ab+c/g;
const div = a / b;
const tpl = `hi ${name} there`;
";

#[test]
fn js_spans_names_and_counts() {
    let a = ScriptAdapter.analyze("t.js", JS);
    assert_eq!(a.error, None, "{:?}", a.error);
    let names: Vec<&str> = a.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"add"), "{names:?}");
    assert!(names.contains(&"twice"), "{names:?}");
    assert!(names.contains(&"Calc::pick"), "{names:?}");

    let add = find(&a.functions, "add");
    assert_eq!((add.start_line, add.end_line), (1, 3));
    assert_eq!(add.cyclomatic, 1);
    assert_eq!(add.loc, 1);
    assert_eq!(add.language, halmi_core::report::Language::JavaScript);

    let pick = find(&a.functions, "Calc::pick");
    assert_eq!(pick.cyclomatic, 2);
    assert_eq!(pick.loc, 4);
    assert_eq!(pick.nesting, 1);
}

#[test]
fn regex_and_division_are_distinguished() {
    let a = ScriptAdapter.analyze("t.js", JS);
    assert_eq!(a.error, None);
    let twice = find(&a.functions, "twice");
    assert_eq!(twice.cyclomatic, 1);
    assert!(twice.volume > 0.0);
}

#[test]
fn ts_types_and_return_annotations() {
    let src = "\
interface Point { x: number; y: number; }

function dist(a: Point, b: Point): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

type Fn = (n: number) => number;
const inc: Fn = (n) => n + 1;
";
    let a = ScriptAdapter.analyze("t.ts", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let dist = find(&a.functions, "dist");
    assert_eq!(dist.language, halmi_core::report::Language::TypeScript);
    assert_eq!(dist.cyclomatic, 1);
    assert!(dist.potential_volume > 0.0);
}

#[test]
fn tsx_jsx_is_lexed() {
    let src = "\
function App({ name }: { name: string }) {
  return <div className=\"x\">{name}</div>;
}
";
    let a = ScriptAdapter.analyze("t.tsx", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let app = find(&a.functions, "App");
    assert_eq!(app.language, halmi_core::report::Language::TypeScript);
    assert!(app.volume > 0.0);
}

#[test]
fn shared_pipeline_and_determinism() {
    let mut functions = ScriptAdapter.analyze("t.js", JS).functions;
    let stats = finalize(&mut functions, &PriorityConfig::default());
    assert!(stats.meaningful >= 2);
    let mut again = ScriptAdapter.analyze("t.js", JS).functions;
    finalize(&mut again, &PriorityConfig::default());
    let names: Vec<_> = functions.iter().map(|r| r.name.as_str()).collect();
    let names2: Vec<_> = again.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, names2);
}

#[test]
fn degraded_files_never_panic() {
    for (path, bad) in [
        ("bad.js", "const s = 'never closed\n"),
        ("bad.js", "const t = `never closed\n"),
        ("bad.tsx", "const x = <div>never closed\n"),
        ("bad.js", "/* never closed\n"),
    ] {
        let a = ScriptAdapter.analyze(path, bad);
        let _ = &a.functions;
    }
}

#[test]
fn unicode_identifiers_are_lexed() {
    let src = "function würstchen(über) {\n  return über + 1;\n}\n";
    let a = ScriptAdapter.analyze("t.js", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(a.functions.len(), 1);
}

#[test]
fn separators_are_not_operators() {
    let src = "function f(a, b) {\n  const o = { x: 1, y: 2 };\n  return a + b;\n}\n";
    let a = ScriptAdapter.analyze("t.js", src);
    let f = find(&a.functions, "f");
    assert!(
        (f.volume - 12.0 * 12.0f64.log2()).abs() < 1e-3,
        "{}",
        f.volume
    );
}

#[test]
fn arrow_body_is_a_block_for_regex_context() {
    let src = "const f = () => {};\nconst re = /ab+c/;\nfunction g() { return 1; }\n";
    let a = ScriptAdapter.analyze("t.js", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(
        a.functions.len(),
        2,
        "{:?}",
        a.functions
            .iter()
            .map(|f| f.name.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn jsx_tags_with_generic_type_arguments_lex() {
    let src = "function A() {\n  return (\n    <div>\n      <Segmented<ProjectExecutionTarget> value={1} />\n    </div>\n  )\n}\n";
    let a = ScriptAdapter.analyze("t.tsx", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(a.functions.len(), 1);
}

#[test]
fn function_and_separator_volume_is_exact() {
    let src = "function add(a, b) {\n  return a + b;\n}\n";
    let a = ScriptAdapter.analyze("t.js", src);
    let f = find(&a.functions, "add");
    assert!(
        (f.volume - 5.0 * 5.0f64.log2()).abs() < 1e-3,
        "{}",
        f.volume
    );
}

#[test]
fn type_annotation_arrows_not_counted() {
    let src = "export function fit(prefix: string, fits: (text: string) => boolean): string {\n    return prefix;\n}\nconst cb: (n: number) => void = null;\nconst obj = { onClick: (e) => e.target };\n";
    let a = ScriptAdapter.analyze("t.ts", src);
    let names: Vec<&str> = a.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"fit"), "{names:?}");
    assert_eq!(
        names.iter().filter(|n| **n == "<arrow>").count(),
        2,
        "value arrows (object literal + typed const) count: {names:?}"
    );
    assert_eq!(names.len(), 3, "type-annotation arrows excluded: {names:?}");
}
