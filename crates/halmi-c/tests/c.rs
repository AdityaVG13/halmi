//! The adapter-port harness plus C-specific edges.

use halmi_c::CAdapter;
use halmi_core::adapter::LanguageAdapter;
use halmi_core::report::FunctionReport;

fn names(a: &halmi_core::adapter::SourceAnalysis) -> Vec<&str> {
    a.functions.iter().map(|f| f.name.as_str()).collect()
}

fn find<'a>(reports: &'a [FunctionReport], name: &str) -> &'a FunctionReport {
    reports
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("{name} not found"))
}

const SRC: &str = r#"
#include <stdio.h>
#define UNUSED(x) ((void)(x))

int add(int a, int b) {
    return a + b;
}

static void helper(double *out, int n) {
    if (out == NULL || n <= 0) {
        return;
    }
    for (int i = 0; i < n; i++) {
        out[i] *= 2.0;
    }
}

char *concat(const char *a, const char *b);

inline int square(int v) { return v * v; }

typedef int (*cmp_fn)(const void *, const void *);

int main(void) {
    printf("%d\n", add(1, 2));
    return 0;
}

struct Point {
    double x;
};

double dist(struct Point *p) {
    return p->x * p->x;
}

int kr(a, b)
int a;
char b;
{
    return a + (int)b;
}
"#;

#[test]
fn definitions_only_with_correct_spans() {
    let a = CAdapter.analyze("t.c", SRC);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(
        names(&a),
        vec!["add", "helper", "square", "main", "dist", "kr"],
        "{:?}",
        names(&a)
    );

    assert_eq!(
        (
            find(&a.functions, "add").start_line,
            find(&a.functions, "add").end_line
        ),
        (5, 7)
    );
    let helper = find(&a.functions, "helper");
    assert_eq!(
        (helper.start_line, helper.end_line),
        (9, 16),
        "storage class + return type folded in"
    );
    assert_eq!(helper.cyclomatic, 4, "1 + if + || + for");
    assert_eq!(
        (
            find(&a.functions, "square").start_line,
            find(&a.functions, "square").end_line
        ),
        (20, 20)
    );
    assert_eq!(
        (
            find(&a.functions, "kr").start_line,
            find(&a.functions, "kr").end_line
        ),
        (37, 42),
        "K&R signature ends at the body brace"
    );
}

#[test]
fn calls_and_prototypes_are_not_definitions() {
    let a = CAdapter.analyze(
        "t.c",
        "int foo(int x);\nint bar(int x) { return foo(x); }\n",
    );
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(names(&a), vec!["bar"]);
}

#[test]
fn function_pointer_typedefs_skipped() {
    let src = "typedef void (*handler)(int sig);\nvoid install(handler h) { }\n";
    let a = CAdapter.analyze("t.c", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(names(&a), vec!["install"]);
}

#[test]
fn preprocessor_macros_not_counted() {
    let src = "#define MAX(a, b) ((a) > (b) ? (a) : (b))\nint real(void) { return MAX(1, 2); }\n";
    let a = CAdapter.analyze("t.c", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(names(&a), vec!["real"]);
}

#[test]
fn lexer_errors_surfaced() {
    let a = CAdapter.analyze("t.c", "const char *s = \"oops\n;\n");
    assert!(a.error.is_some(), "newline in string must error");
}

#[test]
fn method_chain_on_pointer_calls() {
    let src = "int f(void) {\n    return calc(op(x)) + 1;\n}\n";
    let a = CAdapter.analyze("t.c", src);
    let r = find(&a.functions, "f");
    assert_eq!(r.method_chain, 0, "calc(op(x)) is not a dotted chain");
}

#[test]
fn prototype_then_enum_not_a_definition() {
    let src =
        "extern void abort(void);\n\nenum E {\n    A, B\n};\n\nint real(int x) { return x; }\n";
    let a = CAdapter.analyze("t.c", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(names(&a), vec!["real"]);
    let r = find(&a.functions, "real");
    assert_eq!((r.start_line, r.end_line), (7, 7));
}
