//! The adapter-port harness plus Kotlin-specific edges.

use halmi_core::adapter::LanguageAdapter;
use halmi_core::report::FunctionReport;
use halmi_kotlin::KotlinAdapter;

fn names(a: &halmi_core::adapter::SourceAnalysis) -> Vec<String> {
    a.functions.iter().map(|f| f.name.clone()).collect()
}

fn find<'a>(reports: &'a [FunctionReport], name: &str) -> &'a FunctionReport {
    reports
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("{name} not found in {:?}", names_of(reports)))
}

fn names_of(reports: &[FunctionReport]) -> Vec<String> {
    reports.iter().map(|f| f.name.clone()).collect()
}

const SRC: &str = r#"
package demo

@Deprecated("old")
class Widget(private val size: Int) : Base(size), Shape {
    private var extra: Int = 0

    constructor() : this(0)

    init {
        extra = 1
    }

    fun area(): Int {
        return if (size > 0 && size < 100) size * size else 0
    }

    fun doubled() = size * 2

    private inline fun <reified T : Any> cast(x: Any): T? = x as? T

    companion object {
        @JvmStatic
        fun make(): Widget = Widget(1)
    }
}

fun String.shout(): String = this.uppercase() + "!"

fun topLevel(x: Int, y: Int = 2): Int {
    val f = { z: Int -> z + 1 }
    return x + y + f(0)
}

object Singleton {
    fun help() { }
}

data class P(val x: Int, val y: Int) {
    fun norm() = x + y
}

enum class E {
    A, B;

    fun label() = ordinal
}

interface Cb {
    fun onDone(code: Int)
    fun dflt() { }
}
"#;

#[test]
fn fun_declarations_and_qualification() {
    let a = KotlinAdapter.analyze("T.kt", SRC);
    assert_eq!(a.error, None, "{:?}", a.error);
    let n = names(&a);
    for expected in [
        "Widget::constructor",
        "Widget::area",
        "Widget::doubled",
        "Widget::cast",
        "Widget::Companion::make",
        "shout",
        "topLevel",
        "Singleton::help",
        "P::norm",
        "E::label",
        "Cb::onDone",
        "Cb::dflt",
    ] {
        assert!(
            n.contains(&expected.to_string()),
            "{expected} missing from {n:?}"
        );
    }
    assert_eq!(n.len(), 12, "init blocks and lambdas not counted: {n:?}");

    let area = find(&a.functions, "Widget::area");
    assert_eq!((area.start_line, area.end_line), (14, 16));
    assert_eq!(area.cyclomatic, 3, "1 + if + &&");

    let ctor = find(&a.functions, "Widget::constructor");
    assert_eq!((ctor.start_line, ctor.end_line), (8, 8));

    let make = find(&a.functions, "Widget::Companion::make");
    assert_eq!(
        (make.start_line, make.end_line),
        (23, 24),
        "@JvmStatic folded"
    );
}

#[test]
fn top_level_fun_and_receiver() {
    let a = KotlinAdapter.analyze("T.kt", SRC);
    let s = find(&a.functions, "shout");
    assert_eq!(
        (s.start_line, s.end_line),
        (28, 28),
        "receiver type does not rename"
    );
    let t = find(&a.functions, "topLevel");
    assert_eq!(
        (t.start_line, t.end_line),
        (30, 33),
        "expression ends at newline"
    );
}

#[test]
fn string_templates_lex() {
    let src = "fun greet(name: String) = \"hi ${name.uppercase()} and $name!\"\nfun raw() = \"\"\"\n    multi \"line\"\n    \"\"\"\n";
    let a = KotlinAdapter.analyze("T.kt", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let g = find(&a.functions, "greet");
    assert_eq!((g.start_line, g.end_line), (1, 1));
    let r = find(&a.functions, "raw");
    assert_eq!((r.start_line, r.end_line), (2, 4));
}

#[test]
fn expression_body_stops_before_a_modifier() {
    let src = "fun first() =\n    1\noperator fun second() = 2\n";
    let a = KotlinAdapter.analyze("T.kt", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let first = find(&a.functions, "first");
    assert_eq!((first.start_line, first.end_line), (1, 2));
    let second = find(&a.functions, "second");
    assert_eq!((second.start_line, second.end_line), (3, 3));
}

#[test]
fn when_expressions_add_cc() {
    let src = "fun pick(x: Int): Int {\n    return when (x) {\n        1 -> 10\n        2 -> 20\n        else -> 0\n    }\n}\n";
    let a = KotlinAdapter.analyze("T.kt", src);
    let p = find(&a.functions, "pick");
    assert_eq!(p.cyclomatic, 2, "1 + when");
}

#[test]
fn nested_block_comments_do_not_expose_declarations() {
    let src = "/* outer /* nested */ fun fake() = 1\n*/\nfun real() = 2\n";
    let a = KotlinAdapter.analyze("T.kt", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(names(&a), ["real"]);
}

#[test]
fn lexer_errors_surfaced() {
    let a = KotlinAdapter.analyze("T.kt", "val s = \"oops\n");
    assert!(a.error.is_some(), "newline in string must error");
}
