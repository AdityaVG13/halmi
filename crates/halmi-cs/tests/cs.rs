//! The adapter-port harness plus C#-specific edges.

use halmi_core::adapter::LanguageAdapter;
use halmi_core::report::FunctionReport;
use halmi_cs::CsAdapter;

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
using System;

namespace Demo;

[Obsolete("old")]
public sealed class Widget<T> where T : IComparable<T> {
    private int size_;

    public Widget(int size) => size_ = size;

    [STAThread]
    public int Area() {
        if (size_ > 0 && size_ < 100) {
            return size_ * size_;
        }
        return 0;
    }

    public async System.Threading.Tasks.Task<int> FetchAsync(int id) {
        await System.Threading.Tasks.Task.Delay(1);
        return id;
    }

    private string Build(string name) => $"hi {name} {size_:N0}";

    public int Size { get; set; }

    public int Doubled => size_ * 2;

    public void Verbs() {
        var xs = new System.Collections.Generic.List<int> { 1, 2, 3 };
        int Local(int y) => y * 2;
        var z = Local(4);
    }
}

public struct Point {
    public double Norm() => 1.0;
}

public interface Callback {
    void OnDone(int code);
}

public record Point3(int X, int Y) {
    public int Sum() => X + Y;
}

public class Impl : Callback {
    public void OnDone(int code) { }
}
"#;

#[test]
fn methods_ctors_and_expression_bodies() {
    let a = CsAdapter.analyze("T.cs", SRC);
    assert_eq!(a.error, None, "{:?}", a.error);
    let n = names(&a);
    for expected in [
        "Widget::Widget",
        "Widget::Area",
        "Widget::FetchAsync",
        "Widget::Build",
        "Widget::Verbs",
        "Widget::Local",
        "Point::Norm",
        "Callback::OnDone",
        "Point3::Sum",
        "Impl::OnDone",
    ] {
        assert!(
            n.contains(&expected.to_string()),
            "{expected} missing from {n:?}"
        );
    }
    assert!(
        !n.iter()
            .any(|f| f.contains("Size") || f.contains("Doubled")),
        "properties excluded: {n:?}"
    );

    let area = find(&a.functions, "Widget::Area");
    assert_eq!(
        (area.start_line, area.end_line),
        (12, 18),
        "attribute run folded into span"
    );
    assert_eq!(area.cyclomatic, 3, "1 + if + &&");

    let ctor = find(&a.functions, "Widget::Widget");
    assert_eq!(
        (ctor.start_line, ctor.end_line),
        (10, 10),
        "expression-bodied ctor"
    );

    let build = find(&a.functions, "Widget::Build");
    assert_eq!(
        (build.start_line, build.end_line),
        (25, 25),
        "interpolated string is one token"
    );

    let local = find(&a.functions, "Widget::Local");
    assert_eq!(
        (local.start_line, local.end_line),
        (33, 33),
        "local functions count"
    );

    let src = r#"
using System;
namespace N {
  class C {
    [MyCustom("x"), Other]
    public int Count => 1;

    [HttpGet("a")] [Authorize]
    void Serve() { }

    [Serializable]
    class Inner { void M() { } }
  }
}
"#;
    let a = CsAdapter.analyze("attrs.cs", src);
    assert!(a.error.is_none(), "{:?}", a.error);
    let spans: Vec<(&str, u32, u32)> = a
        .functions
        .iter()
        .map(|f| (f.name.as_str(), f.start_line, f.end_line))
        .collect();
    assert_eq!(
        spans,
        [("C::Serve", 8, 9), ("C::Inner::M", 12, 12)],
        "custom attribute names, arguments and stacked groups keep owner and span; `=> 1;` is a property"
    );
}

#[test]
fn file_scoped_namespace_qualifies() {
    let a = CsAdapter.analyze("T.cs", SRC);
    assert!(
        names(&a)
            .iter()
            .all(|f| !f.starts_with("Demo::") || f == "Demo"),
        "file-scoped namespace is not a name prefix"
    );
}

#[test]
fn verbatim_and_raw_strings() {
    let src = "class T {\n    string Path() {\n        return @\"C:\\temp\\\"\"x\"\"\";\n    }\n    string Raw() {\n        return \"\"\"\n            line \"one\"\n            line two\n            \"\"\";\n    }\n}\n";
    let a = CsAdapter.analyze("T.cs", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let n = names(&a);
    assert!(n.contains(&"T::Path".to_string()), "{n:?}");
    assert!(n.contains(&"T::Raw".to_string()), "{n:?}");
    let raw = find(&a.functions, "T::Raw");
    assert_eq!((raw.start_line, raw.end_line), (5, 10));
}

#[test]
fn calls_in_bodies_rejected() {
    let src = "class T {\n    int Work(int x) {\n        Helper(x);\n        return x;\n    }\n    void Helper(int v) { }\n}\n";
    let a = CsAdapter.analyze("T.cs", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let n = names(&a);
    assert_eq!(n.len(), 2, "{n:?}");
    assert!(n.contains(&"T::Work".to_string()));
    assert!(n.contains(&"T::Helper".to_string()));
}

#[test]
fn enum_members_not_methods() {
    let src = "enum Color {\n    Red,\n    Green\n}\nclass T {\n    void F() { }\n}\n";
    let a = CsAdapter.analyze("T.cs", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(names(&a), vec!["T::F".to_string()]);
}

#[test]
fn object_initializers_in_expression_bodies() {
    let src = "class T {\n    System.Collections.Generic.List<int> Make() => new System.Collections.Generic.List<int> { 1, 2 };\n}\n";
    let a = CsAdapter.analyze("T.cs", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let m = find(&a.functions, "T::Make");
    assert_eq!((m.start_line, m.end_line), (2, 2));
}

#[test]
fn lexer_errors_surfaced() {
    let a = CsAdapter.analyze("T.cs", "class T { string s = \"oops\n; }\n");
    assert!(a.error.is_some(), "newline in string must error");
}
