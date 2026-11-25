#![allow(
    clippy::suboptimal_flops,
    reason = "reference values restate the published a*b + c formulas on purpose; fused multiply-add would make the oracle round differently from the implementation"
)]
//! The adapter-port harness plus Go-specific edges.

use halmi_core::adapter::LanguageAdapter;
use halmi_core::ranking::{PriorityConfig, finalize};
use halmi_go::GoAdapter;

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
package main

func add(a int, b int) int {
	return a + b
}

type Server struct{}

func (s *Server) handle(req string, n int) (int, error) {
	// decide
	if n > 1 && req != \"\" {
		return n * 2, nil
	}
	for i := 0; i < n; i++ {
		n += i
	}
	return 0, nil
}

func chained(s string) []string {
	return strings.ToLower(strings.TrimSpace(s))
}
";

#[test]
fn spans_names_and_counts() {
    let a = GoAdapter.analyze("t.go", SRC);
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
    assert_eq!((add.start_line, add.end_line), (3, 5));
    assert_eq!(add.cyclomatic, 1);
    assert_eq!(add.loc, 1);

    let handle = find(&a.functions, "Server::handle");
    assert_eq!((handle.start_line, handle.end_line), (9, 18));
    assert_eq!(handle.cyclomatic, 4, "1 + if + && + for");
    assert_eq!(handle.nesting, 1, "if and for are sibling bodies");

    let chained = find(&a.functions, "chained");
    assert_eq!(
        chained.method_chain, 1,
        "single method call on the package name"
    );
}

#[test]
fn raw_strings_and_runes() {
    let src = "func f() string {\n\tre := `a /* not comment */ b`\n\tq := '/'\n\treturn re\n}\n";
    let a = GoAdapter.analyze("t.go", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let f = find(&a.functions, "f");
    assert_eq!(f.cyclomatic, 1);
    assert!(f.mi_sei.is_some());
}

#[test]
fn shared_pipeline_and_determinism() {
    let mut functions = GoAdapter.analyze("t.go", SRC).functions;
    let stats = finalize(&mut functions, &PriorityConfig::default());
    assert!(stats.meaningful >= 2);
    let mut again = GoAdapter.analyze("t.go", SRC).functions;
    finalize(&mut again, &PriorityConfig::default());
    let names: Vec<_> = functions.iter().map(|r| r.name.as_str()).collect();
    let names2: Vec<_> = again.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, names2);
}

#[test]
fn raw_strings_do_not_drift_line_numbers() {
    let src = "func f() string {\n\tr := `a\nb`\n\t// after raw\n\treturn r\n}\n";
    let a = GoAdapter.analyze("t.go", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let f = find(&a.functions, "f");
    assert_eq!((f.start_line, f.end_line), (1, 6));
}

#[test]
fn degraded_files_never_panic() {
    for bad in [
        "func broken( {\n",
        "s := \"never closed\n",
        "c := `open raw\n",
    ] {
        let a = GoAdapter.analyze("bad.go", bad);
        let _ = &a.functions;
    }
}

#[test]
fn func_types_in_type_block_are_not_declarations() {
    let src = "package main\n\ntype (\n\ta func(int) int\n\tb func(int, *string) (bool, error)\n\tc func(int, ...*string) bool\n)\n\nfunc real(x int) int { return x }\n";
    let a = GoAdapter.analyze("types.go", src);
    let names: Vec<&str> = a.functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["real"]);
}

#[test]
fn interface_and_struct_literal_return_types_keep_the_body() {
    let src = "package main\n\nfunc m(id int) map[string]interface{} {\n\treturn nil\n}\n\nfunc (p Pair) boxed() struct{ a, b int } {\n\treturn Pair{1, 2}.boxed()\n}\n\nfunc typed() interface{ error } {\n\treturn nil\n}\n";
    let a = GoAdapter.analyze("ret.go", src);
    for (name, end) in [("m", 5), ("Pair::boxed", 9), ("typed", 13)] {
        let r = find(&a.functions, name);
        assert_eq!((r.start_line, r.end_line), (r.start_line, end), "{name}");
    }
}

#[test]
fn io_params_count_signature() {
    let a = GoAdapter.analyze("t.go", SRC);
    let handle = find(&a.functions, "Server::handle");
    let expected = (2.0 + 3.0) * 5.0f64.log2();
    assert!((handle.potential_volume - expected).abs() < 1e-3);
}

#[test]
fn grouped_parameters_count_each_name() {
    let src = "package main\n\nfunc add(a, b int) int {\n\treturn a + b\n}\n";
    let a = GoAdapter.analyze("t.go", src);
    let add = find(&a.functions, "add");
    let expected = (2.0 + 3.0) * 5.0f64.log2();
    assert!(
        (add.potential_volume - expected).abs() < 1e-3,
        "{}",
        add.potential_volume
    );
}

#[test]
fn separators_are_not_operators() {
    let src =
        "package main\n\nfunc f(a int, b int) int {\n\tm := map[int]int{1: 2}\n\treturn a + b\n}\n";
    let a = GoAdapter.analyze("t.go", src);
    let f = find(&a.functions, "f");
    assert!(
        f.volume < 40.0,
        "volume {} suggests separators counted",
        f.volume
    );
}

#[test]
fn func_and_separator_volume_is_exact() {
    let src = "package main\n\nfunc add(a, b int) int {\n\treturn a + b\n}\n";
    let a = GoAdapter.analyze("t.go", src);
    let add = find(&a.functions, "add");
    assert!(
        (add.volume - 5.0 * 5.0f64.log2()).abs() < 1e-3,
        "{}",
        add.volume
    );
}

#[test]
fn body_less_declarations_end_at_their_signature_line() {
    let src = "package sync\n\n// Semacquire waits until *s > 0.\nfunc runtime_Semacquire(s *uint32)\nfunc runtime_SemacquireRWMutexR(s *uint32, lifo bool, skipframes int)\n\nfunc runtime_notifyListAdd(l *notifyList) uint32\n\nfunc init() {\n\tvar n notifyList\n}\n\nfunc g1[T any](a T) {}\nfunc (p *T1) M1()";
    let a = GoAdapter.analyze("stubs.go", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let spans: Vec<(&str, u32, u32)> = a
        .functions
        .iter()
        .map(|f| (f.name.as_str(), f.start_line, f.end_line))
        .collect();
    assert_eq!(
        spans,
        [("init", 9, 11), ("g1", 13, 13)],
        "a body-less (assembly-implemented) declaration is a signature, not a definition: not counted, and it must not swallow the declarations after it"
    );
}
