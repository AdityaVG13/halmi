//! The adapter-port harness plus Swift-specific edges.

use halmi_core::adapter::LanguageAdapter;
use halmi_core::ranking::{PriorityConfig, finalize};
use halmi_swift::SwiftAdapter;

fn find<'a>(
    reports: &'a [halmi_core::report::FunctionReport],
    name: &str,
) -> &'a halmi_core::report::FunctionReport {
    reports
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("{name} not found in {reports:?}"))
}

const SRC: &str = r#"
import Foundation

@available(iOS 13, *)
public final class Calc {
    private var total: Double = 0

    public init(start: Double) {
        total = start
    }

    deinit {
        print("bye \(total)")
    }

    func pick(_ x: Double) -> Double {
        if x > 1 {
            return x * 2
        }
        return 0
    }

    func chained(_ v: String) -> String {
        return v.trimmingCharacters(in: .whitespaces).lowercased()
    }

    var desc: String {
        "total \(total)"
    }
}

protocol Shape {
    func area() -> Double
}

func free(n: Int) -> Int {
    for i in 0..<n where i % 2 == 0 {
        if i > 2 { continue }
    }
    return n
}
"#;

#[test]
fn spans_names_and_counts() {
    let a = SwiftAdapter.analyze("t.swift", SRC);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(
        a.functions.len(),
        5,
        "{:?}",
        a.functions
            .iter()
            .map(|f| f.name.clone())
            .collect::<Vec<_>>()
    );

    let init_fn = find(&a.functions, "Calc::init");
    assert_eq!(
        (init_fn.start_line, init_fn.end_line),
        (8, 10),
        "modifiers included in span"
    );
    let deinit_fn = find(&a.functions, "Calc::deinit");
    assert_eq!((deinit_fn.start_line, deinit_fn.end_line), (12, 14));

    let pick = find(&a.functions, "Calc::pick");
    assert_eq!((pick.start_line, pick.end_line), (16, 21));
    assert_eq!(pick.cyclomatic, 2);
    assert_eq!(
        pick.nesting, 1,
        "if body is one level under the function body"
    );
    assert_eq!(pick.loc, 4, "body lines only, like the Go adapter");

    let chained = find(&a.functions, "Calc::chained");
    assert_eq!(
        chained.method_chain, 2,
        "trimmingCharacters() fresh, lowercased() extends"
    );

    assert!(
        !a.functions.iter().any(|f| f.name == "Shape::area"),
        "bodyless protocol members are not counted, matching the Rust and Go adapters"
    );

    let free = find(&a.functions, "free");
    assert_eq!((free.start_line, free.end_line), (36, 41));
    assert_eq!(free.cyclomatic, 4, "1 + for + where + if");
    assert_eq!(free.loc, 4);

    let mut functions = a.functions.clone();
    let stats = finalize(&mut functions, &PriorityConfig::default());
    assert!(stats.meaningful >= 2);
    assert!(functions.iter().all(|r| !r.reason.is_empty()));
}

#[test]
fn strings_interpolation_and_multiline() {
    let src = r##"
func greet(_ who: String) -> String {
    let raw = #"a "quote" here"#
    let multi = """
        line one \(who.count)
        line two
        """
    let nested = "outer \(inner("x \(who)")) end"
    return multi + raw + nested
}
"##;
    let a = SwiftAdapter.analyze("s.swift", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(a.functions.len(), 1, "{:?}", a.error);
    let f = find(&a.functions, "greet");
    assert_eq!(f.end_line, 10);
}

#[test]
fn modifier_on_its_own_line_starts_the_span() {
    let src = r"
func plain() {}

@discardableResult
private
func decorated() -> Int {
    return 1
}
";
    let a = SwiftAdapter.analyze("m.swift", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let plain = find(&a.functions, "plain");
    assert_eq!((plain.start_line, plain.end_line), (2, 2));
    let decorated = find(&a.functions, "decorated");
    assert_eq!(
        (decorated.start_line, decorated.end_line),
        (4, 8),
        "attribute and modifier lines lead the span"
    );
}

#[test]
fn backtick_names_and_operators() {
    let src = r"
struct Ops {
    static func == (lhs: Ops, rhs: Ops) -> Bool {
        return lhs === rhs
    }
    func `default`(_ x: Int) -> Int {
        return x ?? 0
    }
}
";
    let a = SwiftAdapter.analyze("o.swift", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let eq = find(&a.functions, "Ops::==");
    assert_eq!((eq.start_line, eq.end_line), (3, 5));
    let def = find(&a.functions, "Ops::default");
    assert_eq!((def.start_line, def.end_line), (6, 8));
}

#[test]
fn block_comments_nest_and_terminate() {
    let src = r"
func documented() -> Int {
    /* outer /* nested */ still outer */
    return 1
    /*
    trailing multi-line comment
    */
}
";
    let a = SwiftAdapter.analyze("c.swift", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(a.functions.len(), 1);
}

#[test]
fn bad_source_degrades_to_error() {
    let a = SwiftAdapter.analyze("bad.swift", "func broken( { \"unterminated");
    assert!(a.error.is_some());
    assert_eq!(a.functions, [] as [halmi_core::FunctionReport; 0]);
}

#[test]
fn unterminated_generic_list_does_not_panic() {
    let src = "func f<>>(x: Int) {}\nfunc g(x: Int) -> Int { return x }\n";
    let a = SwiftAdapter.analyze("g.swift", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let found: Vec<&str> = a.functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(found, vec!["f", "g"]);
}
