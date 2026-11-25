//! The adapter-port harness plus C++-specific edges.

use halmi_core::adapter::LanguageAdapter;
use halmi_core::report::FunctionReport;
use halmi_cpp::CppAdapter;

fn names(a: &halmi_core::adapter::SourceAnalysis) -> Vec<&str> {
    a.functions.iter().map(|f| f.name.as_str()).collect()
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

const SRC: &str = r"
#include <vector>
#include <string>

namespace demo {

class Widget {
public:
    Widget(int size) : size_(size), buf_(size) {}
    ~Widget() = default;

    int area() const noexcept { return size_ * size_; }
    virtual void draw() const {}
    Widget& operator+=(const Widget& other) { size_ += other.size_; return *this; }
    bool operator==(const Widget& o) const { return size_ == o.size_; }
    int operator[](int i) const { return i; }
    void call_op() { (*this)(); }

protected:
    int size_ = 0;
    std::vector<int> buf_;
private:
    void helper();
};

void Widget::helper() {}

struct Node {
    Node* next = nullptr;
    static int count() { return 0; }
};

}  // namespace demo

template <typename T, int N>
T sumArray(T (&arr)[N]) {
    T total = T{};
    for (int i = 0; i < N; ++i) {
        total += arr[i];
    }
    return total;
}

inline auto trailing(const std::string& s) -> size_t {
    return s.size();
}

int main() {
    Widget w{3};
    auto lam = [](int x) { return x * 2; };
    return lam(w.area());
}
";

#[test]
fn methods_constructors_and_quals() {
    let a = CppAdapter.analyze("t.cpp", SRC);
    assert_eq!(a.error, None, "{:?}", a.error);
    let n = names(&a);
    for expected in [
        "Widget::Widget",
        "Widget::~Widget",
        "Widget::area",
        "Widget::draw",
        "Widget::operator+=",
        "Widget::operator==",
        "Widget::operator[]",
        "Widget::call_op",
        "Widget::helper",
        "Node::count",
        "sumArray",
        "trailing",
        "main",
    ] {
        assert!(n.contains(&expected), "{expected} missing from {n:?}");
    }
    assert_eq!(n.len(), 13, "lambdas and prototypes not counted: {n:?}");

    assert_eq!(
        (
            find(&a.functions, "sumArray").start_line,
            find(&a.functions, "sumArray").end_line
        ),
        (36, 42),
        "return type line through body (template header excluded, matching tree-sitter)"
    );
    assert_eq!(
        (
            find(&a.functions, "Widget::Widget").start_line,
            find(&a.functions, "Widget::Widget").end_line
        ),
        (9, 9),
        "init list on one line"
    );
    assert_eq!(
        (
            find(&a.functions, "Widget::~Widget").start_line,
            find(&a.functions, "Widget::~Widget").end_line
        ),
        (10, 10),
        "= default counts"
    );
    assert_eq!(
        (
            find(&a.functions, "main").start_line,
            find(&a.functions, "main").end_line
        ),
        (48, 52)
    );
}

#[test]
fn prototypes_and_lambdas_excluded() {
    let src = "class F {\n    void proto();\n    int use() { auto l = [](int x) { return x; }; return l(1); }\n};\nvoid F::proto() {}\n";
    let a = CppAdapter.analyze("t.cpp", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let n = names(&a);
    assert!(n.contains(&"F::use"), "{n:?}");
    assert!(n.contains(&"F::proto"), "{n:?}");
    assert_eq!(n.len(), 2, "{n:?}");
}

#[test]
fn trailing_return_and_const_suffix() {
    let src = "auto f(int x) const -> int {\n    return x;\n}\n";
    let a = CppAdapter.analyze("t.cpp", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(names(&a), vec!["f"]);
    assert_eq!(
        (
            find(&a.functions, "f").start_line,
            find(&a.functions, "f").end_line
        ),
        (1, 3)
    );
}

#[test]
fn raw_string_and_multiline() {
    let src = "const char* doc() {\n    static const char* d = R\"x(quote \" and\nnewline)x\";\n    return d;\n}\n";
    let a = CppAdapter.analyze("t.cpp", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(names(&a), vec!["doc"]);
    assert_eq!(
        (
            find(&a.functions, "doc").start_line,
            find(&a.functions, "doc").end_line
        ),
        (1, 5)
    );
}

#[test]
fn operator_call_and_index() {
    let a = CppAdapter.analyze("t.cpp", SRC);
    let call = find(&a.functions, "Widget::call_op");
    assert!(call.cyclomatic >= 1);
}

#[test]
fn digit_separators() {
    let src = "int big() {\n    int m = 1'000'000;\n    return m;\n}\n";
    let a = CppAdapter.analyze("t.cpp", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(names(&a), vec!["big"]);
}

#[test]
fn lexer_errors_surfaced() {
    let a = CppAdapter.analyze("t.cpp", "const char* s = \"oops\n;\n");
    assert!(a.error.is_some(), "newline in string must error");
}

#[test]
fn namespace_container_qualifies() {
    let a = CppAdapter.analyze("t.cpp", SRC);
    assert!(
        names(&a).contains(&"Widget::helper"),
        "out-of-line method keeps written scope"
    );
    let helper = find(&a.functions, "Widget::helper");
    assert_eq!((helper.start_line, helper.end_line), (26, 26));
}

#[test]
fn template_specializations_counted() {
    let src = "template <> void net<4>(int* v) { sort4(v); }\ntemplate <> void net<6>(int* v) { sort6(v); }\n";
    let a = CppAdapter.analyze("t.cpp", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(names(&a), vec!["net", "net"]);
    assert_eq!(
        (
            find(&a.functions, "net").start_line,
            find(&a.functions, "net").end_line
        ),
        (1, 1)
    );
}

#[test]
fn shift_after_less_than_does_not_panic() {
    let src = "static const int k = FLAGS < (MASK >> 2);\nint f() { return k; }\n";
    let a = CppAdapter.analyze("t.cpp", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(names(&a), vec!["f"]);
}
