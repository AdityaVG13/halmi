//! The adapter-port harness plus Lua-specific edges.

use halmi_core::adapter::LanguageAdapter;
use halmi_core::report::FunctionReport;
use halmi_lua::LuaAdapter;

fn names(a: &halmi_core::adapter::SourceAnalysis) -> Vec<String> {
    a.functions.iter().map(|f| f.name.clone()).collect()
}

fn find<'a>(reports: &'a [FunctionReport], name: &str) -> &'a FunctionReport {
    reports
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("{name} not found"))
}

const SRC: &str = r"
local Math = {}

function Math.clamp(x, lo, hi)
    if x < lo then
        return lo
    elseif x > hi then
        return hi
    end
    return x
end

local function helper(name, opts)
    local t = { a = 1 }
    for i = 1, 10 do
        t[i] = i * 2
    end
    return t
end

Widget = {}

function Widget.new(size)
    local self = setmetatable({}, Widget)
    self.size = size or 0
    return self
end

function Widget:area()
    return self.size * self.size
end

function Widget.map(list, fn)
    local function nested(x) return x + 1 end
    return nested(1)
end

local arrow = function(x) return x * 2 end
";

#[test]
fn declarations_and_naming() {
    let a = LuaAdapter.analyze("t.lua", SRC);
    assert_eq!(a.error, None, "{:?}", a.error);
    let n = names(&a);
    for expected in [
        "Math.clamp",
        "helper",
        "Widget.new",
        "Widget:area",
        "Widget.map",
        "nested",
    ] {
        assert!(
            n.contains(&expected.to_string()),
            "{expected} missing: {n:?}"
        );
    }
    assert_eq!(n.len(), 6, "anonymous functions excluded: {n:?}");

    let clamp = find(&a.functions, "Math.clamp");
    assert_eq!((clamp.start_line, clamp.end_line), (4, 11));
    assert_eq!(clamp.cyclomatic, 3, "1 + if + elseif");

    let helper = find(&a.functions, "helper");
    assert_eq!(
        (helper.start_line, helper.end_line),
        (13, 19),
        "local folded"
    );

    let nested = find(&a.functions, "nested");
    assert_eq!(
        (nested.start_line, nested.end_line),
        (34, 34),
        "nested named functions count"
    );
}

#[test]
fn repeat_until_blocks_balance() {
    let src = "function retry(fn)\n  local n = 0\n  repeat\n    n = n + 1\n  until n > 3 or fn()\n  return n\nend\n";
    let a = LuaAdapter.analyze("t.lua", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let r = find(&a.functions, "retry");
    assert_eq!((r.start_line, r.end_line), (1, 7));
    assert_eq!(r.cyclomatic, 2, "1 + until-condition or");
}

#[test]
fn long_strings_and_comments() {
    let src =
        "--[[ config\n block ]]\nlocal C = [[\nraw ]] here\n]]\nfunction f()\n  return C\nend\n";
    let a = LuaAdapter.analyze("t.lua", src);
    assert_eq!(a.error, None, "{:?}", a.error);
    let f = find(&a.functions, "f");
    assert_eq!((f.start_line, f.end_line), (6, 8));
}

#[test]
fn lexer_errors_surfaced() {
    let a = LuaAdapter.analyze("t.lua", "local s = \"oops\n");
    assert!(a.error.is_some(), "newline in string must error");
}
