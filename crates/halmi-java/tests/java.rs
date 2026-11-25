//! The adapter-port harness plus Java-specific edges.

use halmi_core::adapter::LanguageAdapter;
use halmi_core::report::FunctionReport;
use halmi_java::JavaAdapter;

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
package demo;

import java.util.List;

@Deprecated
public final class Widget<T extends Comparable<T>> {
    private static final int SIZE = 3;
    private int size_;

    @Override
    @SuppressWarnings("unchecked")
    public int area() {
        if (size_ > 0 && size_ < 100) {
            return size_ * size_;
        }
        return 0;
    }

    public Widget(int size) {
        this.size_ = size;
    }

    static {
        System.out.println("init");
    }

    public <R> R apply(Function<T, R> f) throws Exception {
        return f.apply((T) this);
    }

    native void fastPath(int x);

    abstract void mustImplement();

    public void lambdas() {
        Runnable r = () -> { System.out.println("x"); };
        xs.forEach(s -> s.length());
    }

    private class Inner {
        void nested() {}
    }

    enum Color {
        RED, GREEN {
            void specialty() {}
        };

        void common() {}
    }

    interface Callback {
        void onDone(int code);
        default void onFail() { onDone(-1); }
    }
}

record Point(int x, int y) {
    double norm() { return Math.hypot(x, y); }
}
"#;

#[test]
fn declarations_spans_and_qualification() {
    let a = JavaAdapter.analyze("T.java", SRC);
    assert_eq!(a.error, None, "{:?}", a.error);
    let n = names(&a);
    for expected in [
        "Widget::area",
        "Widget::Widget",
        "Widget::<clinit>",
        "Widget::apply",
        "Widget::fastPath",
        "Widget::mustImplement",
        "Widget::lambdas",
        "Widget::Inner::nested",
        "Widget::Color::GREEN::specialty",
        "Widget::Color::common",
        "Widget::Callback::onDone",
        "Widget::Callback::onFail",
        "Point::norm",
    ] {
        assert!(
            n.contains(&expected.to_string()),
            "{expected} missing from {n:?}"
        );
    }

    let area = find(&a.functions, "Widget::area");
    assert_eq!(
        (area.start_line, area.end_line),
        (11, 18),
        "annotation run folded into span"
    );
    assert_eq!(area.cyclomatic, 3, "1 + if + &&");

    let ctor = find(&a.functions, "Widget::Widget");
    assert_eq!((ctor.start_line, ctor.end_line), (20, 22));

    let clinit = find(&a.functions, "Widget::<clinit>");
    assert_eq!((clinit.start_line, clinit.end_line), (24, 26));

    let native_m = find(&a.functions, "Widget::fastPath");
    assert_eq!(
        (native_m.start_line, native_m.end_line),
        (32, 32),
        "bodyless native counts"
    );

    let abstract_m = find(&a.functions, "Widget::mustImplement");
    assert_eq!((abstract_m.start_line, abstract_m.end_line), (34, 34));
}

#[test]
fn lambdas_not_counted() {
    let a = JavaAdapter.analyze("T.java", SRC);
    let n = names(&a);
    assert_eq!(n.iter().filter(|f| f.contains('$')).count(), 0);
    let lam = find(&a.functions, "Widget::lambdas");
    assert_eq!((lam.start_line, lam.end_line), (36, 39));
}

#[test]
fn interface_and_enum_bodies() {
    let a = JavaAdapter.analyze("T.java", SRC);
    let on_done = find(&a.functions, "Widget::Callback::onDone");
    assert_eq!((on_done.start_line, on_done.end_line), (54, 54));
    let specialty = find(&a.functions, "Widget::Color::GREEN::specialty");
    assert_eq!(
        (specialty.start_line, specialty.end_line),
        (47, 47),
        "enum constant body"
    );
}

#[test]
fn anonymous_class_methods_count() {
    let src = "class T {\n    Runnable r = new Runnable() {\n        public void run() { go(); }\n    };\n}\n";
    let a = JavaAdapter.analyze("T.java", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let n = names(&a);
    assert!(n.contains(&"T::run".to_string()), "{n:?}");
    assert_eq!(n.len(), 1, "{n:?}");
}

#[test]
fn text_blocks_lex() {
    let src = "class T {\n    String doc() {\n        return \"\"\"\n            hello \"world\"\n            bye\n            \"\"\";\n    }\n}\n";
    let a = JavaAdapter.analyze("T.java", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let d = find(&a.functions, "T::doc");
    assert_eq!((d.start_line, d.end_line), (2, 7));
}

#[test]
fn calls_inside_bodies_rejected() {
    let src = "class T {\n    int work(int x) {\n        helper(x);\n        other(y);\n        return x;\n    }\n    void helper(int v) { }\n}\n";
    let a = JavaAdapter.analyze("T.java", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let n = names(&a);
    assert!(n.contains(&"T::work".to_string()), "{n:?}");
    assert!(n.contains(&"T::helper".to_string()), "{n:?}");
    assert_eq!(n.len(), 2, "{n:?}");
}

#[test]
fn generics_disambiguation() {
    let src = "class Pair<A, B> {\n    <C extends Comparable<C>> C pick(A a, B b) {\n        return null;\n    }\n}\n";
    let a = JavaAdapter.analyze("T.java", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let p = find(&a.functions, "Pair::pick");
    assert_eq!((p.start_line, p.end_line), (2, 4));
}

#[test]
fn lexer_errors_surfaced() {
    let a = JavaAdapter.analyze("T.java", "class T { String s = \"oops\n; }\n");
    assert!(a.error.is_some(), "newline in string must error");
}

#[test]
fn enum_constant_with_args_is_not_a_method() {
    let src = "enum E {\n    A(1) {\n        void f() {}\n    },\n    B(2);\n\n    E(int x) { }\n\n    void common() { }\n}\n";
    let a = JavaAdapter.analyze("T.java", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let n = names(&a);
    assert!(n.contains(&"E::A::f".to_string()), "{n:?}");
    assert!(n.contains(&"E::E".to_string()), "{n:?}");
    assert!(n.contains(&"E::common".to_string()), "{n:?}");
    assert!(!n.iter().any(|f| f == "E::A" || f == "E::B"), "{n:?}");
}
