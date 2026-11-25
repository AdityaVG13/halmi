//! The adapter-port harness plus Zig-specific edges.

use halmi_core::adapter::LanguageAdapter;
use halmi_core::report::FunctionReport;
use halmi_zig::ZigAdapter;

fn find<'a>(reports: &'a [FunctionReport], name: &str) -> &'a FunctionReport {
    reports
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("{name} not found in {:?}", report_names(reports)))
}

fn report_names(reports: &[FunctionReport]) -> Vec<&str> {
    reports.iter().map(|f| f.name.as_str()).collect()
}

const SRC: &str = r#"
const std = @import("std");

pub fn add(a: i32, b: i32) i32 {
    return a + b;
}

fn count(lines: []const u8) usize {
    var n: usize = 0;
    for (lines) |ch| {
        if (ch == '\n' and n > 0) {
            n += 1;
        } else if (ch == '\r') {
            continue;
        }
    }
    return n;
}

extern fn external(x: u32) u32;

const Impl = struct {
    fn method(self: *const Impl) void {
        _ = self;
    }

    pub fn pubMethod(comptime T: type, v: T) !T {
        const s =
            \\first line
            \\second line
        ;
        _ = s;
        return v;
    }
};

test "addition" {
    try std.testing.expectEqual(@as(i32, 3), add(1, 2));
}

test {
    _ = count("a\nb");
}

const FnPtr = *const fn (u32) u32;
"#;

#[test]
fn declarations_spans_and_qualification() {
    let a = ZigAdapter.analyze("t.zig", SRC);
    assert_eq!(a.error, None, "{:?}", a.error);
    let names = report_names(&a.functions);
    assert_eq!(
        names,
        vec![
            "add",
            "count",
            "external",
            "Impl::method",
            "Impl::pubMethod",
            "test \"addition\"",
            "test",
        ],
        "{names:?}"
    );

    let add = find(&a.functions, "add");
    assert_eq!((add.start_line, add.end_line), (4, 6));

    let count = find(&a.functions, "count");
    assert_eq!((count.start_line, count.end_line), (8, 18));
    assert_eq!(
        count.cyclomatic, 5,
        "1 + for + if + and + else-if + continue-guard `if`"
    );

    let external = find(&a.functions, "external");
    assert_eq!((external.start_line, external.end_line), (20, 20));
    assert_eq!(external.loc, 0, "bodyless extern has an empty body");

    let method = find(&a.functions, "Impl::method");
    assert_eq!((method.start_line, method.end_line), (23, 25));
    let pub_method = find(&a.functions, "Impl::pubMethod");
    assert_eq!((pub_method.start_line, pub_method.end_line), (27, 34));
    let test_named = find(&a.functions, "test \"addition\"");
    assert_eq!((test_named.start_line, test_named.end_line), (37, 39));

    let test_plain = find(&a.functions, "test");
    assert_eq!((test_plain.start_line, test_plain.end_line), (41, 43));
}

#[test]
fn fn_types_not_counted() {
    let src = "const FnPtr = *const fn (u32) u32;\nfn real() void {}\n";
    let a = ZigAdapter.analyze("t.zig", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(report_names(&a.functions), vec!["real"]);
}

#[test]
fn modifier_run_included_in_span() {
    let src = "\npub\ncallconv(.C)\nfn cfn(x: u8) u8 {\n    return x;\n}\n";
    let a = ZigAdapter.analyze("t.zig", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(report_names(&a.functions), vec!["cfn"]);
    let f = find(&a.functions, "cfn");
    assert_eq!(f.start_line, 2, "span starts at `pub`, not `fn`");
    assert_eq!(f.end_line, 6);
}

#[test]
fn nested_namespace_fn_found() {
    let src = "fn outer() void {\n    const Inner = struct {\n        fn helper() void {}\n    };\n    _ = Inner;\n}\n";
    let a = ZigAdapter.analyze("t.zig", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(report_names(&a.functions), vec!["outer", "Inner::helper"]);
}

#[test]
fn method_chain_and_call_counts() {
    let src =
        "fn chains(v: []const u8) []const u8 {\n    return std.mem.trim(u8, v, \" \").*;\n}\n";
    let a = ZigAdapter.analyze("t.zig", src);
    let f = find(&a.functions, "chains");
    assert_eq!(f.method_chain, 1);
}

#[test]
fn lexer_errors_surfaced() {
    let a = ZigAdapter.analyze("t.zig", "const s = \"oops\n;\n");
    assert!(a.error.is_some(), "newline in string must error");
}

#[test]
fn raw_identifiers_named_correctly() {
    let src = "// header\npub fn @\"odd name\"() void {\n    // inside\n    return;\n}\n";
    let a = ZigAdapter.analyze("t.zig", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    assert_eq!(report_names(&a.functions), vec!["odd name"]);
}
