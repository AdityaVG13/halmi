//! halmi-cpp: C++ language adapter.
//!
//! Hand lexer + definition scanner on [`halmi_lex`] shared machinery
//! (same shape as `halmi-c`, extended for C++ declarations). Zero
//! dependencies, no tree-sitter: the tree-sitter-cpp oracle is a
//! test-side differential only.
//!
//! * Definitions: `function_definition` parity with tree-sitter-cpp --
//!   free functions, methods (in-class and out-of-line
//!   `Type::method`), constructors, destructors (including
//!   `= default;` / `= delete;`), operator overloads, and
//!   `template<...>` headers folded into the span. Prototypes,
//!   lambdas, and pure-virtual `= 0` declarations are not counted.
//! * Qualification: `Container::name` via a class/struct/union/enum/
//!   namespace container stack; out-of-line definitions keep their
//!   written scope chain (`Foo::Bar::method`).
//! * Constructor initializer lists, trailing return types
//!   (`auto f() -> T`), `noexcept(...)`, `override`, `final`,
//!   `requires (...)`, and `const` suffixes are signature tokens.
//! * Preprocessor: single-branch brace model (see the halmi-c
//!   contract); backslash-newline splices anywhere.
//! * `decl_op`: the declarator `(` (C++ has no `fn` keyword).

#![deny(unsafe_code)]

use halmi_core::adapter::{LanguageAdapter, SourceAnalysis};
use halmi_core::report::{Language, RawFunction, count_f64, count_u32};
use halmi_lex::{LangSpec, LocMode, MetricsCollector};

#[derive(Debug, Clone, Copy, Default)]
pub struct CppAdapter;

impl LanguageAdapter for CppAdapter {
    fn language(&self) -> &'static str {
        "Cpp"
    }

    fn analyze(&self, path: &str, src: &str) -> SourceAnalysis {
        match analyze_cpp(src) {
            Ok(mut raws) => {
                for raw in &mut raws {
                    raw.path = path.to_string();
                    raw.language = Language::Cpp;
                }
                SourceAnalysis {
                    path: path.to_string(),
                    functions: self.reports_from_raw(raws),
                    error: None,
                }
            }
            Err(msg) => SourceAnalysis {
                path: path.to_string(),
                functions: Vec::new(),
                error: Some(msg),
            },
        }
    }
}

const CPP_SPEC: LangSpec = LangSpec {
    decl_op: "(",
    keyword_operators: CONTROL_KEYWORDS,
    cc_keywords: &["if", "for", "while", "case", "catch"],
    separator_ops: &["(", ")", "[", "]", "{", "}", ",", ";", ":"],
    cc_ops: &["&&", "||"],
    loc_mode: LocMode::Physical,
};

const CONTROL_KEYWORDS: &[&str] = &[
    "if",
    "else",
    "for",
    "while",
    "do",
    "switch",
    "case",
    "default",
    "break",
    "continue",
    "return",
    "goto",
    "sizeof",
    "new",
    "delete",
    "try",
    "catch",
    "throw",
    "operator",
    "co_return",
    "co_await",
    "co_yield",
];

const ALL_KEYWORDS: &[&str] = &[
    "alignas",
    "alignof",
    "and",
    "asm",
    "auto",
    "bool",
    "break",
    "case",
    "catch",
    "char",
    "char16_t",
    "char32_t",
    "char8_t",
    "class",
    "co_await",
    "co_return",
    "co_yield",
    "concept",
    "const",
    "const_cast",
    "consteval",
    "constexpr",
    "constinit",
    "continue",
    "decltype",
    "default",
    "delete",
    "do",
    "double",
    "dynamic_cast",
    "else",
    "enum",
    "explicit",
    "export",
    "extern",
    "false",
    "float",
    "for",
    "friend",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "mutable",
    "namespace",
    "new",
    "noexcept",
    "nullptr",
    "operator",
    "private",
    "protected",
    "public",
    "register",
    "reinterpret_cast",
    "requires",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "static_assert",
    "static_cast",
    "struct",
    "switch",
    "template",
    "this",
    "thread_local",
    "throw",
    "true",
    "try",
    "typedef",
    "typeid",
    "typename",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "wchar_t",
    "while",
];

const NON_FUNCTIONS: &[&str] = &[
    "__attribute__",
    "__declspec",
    "__asm__",
    "asm",
    "__typeof__",
    "__alignof__",
    "defined",
    "sizeof",
    "_Alignof",
    "offsetof",
    "alignof",
    "decltype",
    "noexcept",
    "static_assert",
    "requires",
    "typeid",
    "assert",
];

const TYPEISH: &[&str] = &[
    "const",
    "constexpr",
    "consteval",
    "constinit",
    "static",
    "extern",
    "inline",
    "register",
    "auto",
    "signed",
    "unsigned",
    "struct",
    "enum",
    "class",
    "union",
    "typedef",
    "typename",
    "long",
    "short",
    "int",
    "char",
    "float",
    "double",
    "void",
    "bool",
    "wchar_t",
    "char8_t",
    "char16_t",
    "char32_t",
    "virtual",
    "explicit",
    "friend",
    "mutable",
    "thread_local",
    "volatile",
    "template",
];

const CONTAINER_KEYWORDS: &[&str] = &["struct", "class", "union", "enum"];

const GAP_KEYWORDS: &[&str] = &["const", "noexcept", "override", "final", "requires"];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok<'a> {
    Ident(&'a str),
    Number(&'a str),
    Str(&'a str),
    Rune(&'a str),
    Op(&'a str),
    Kw(&'a str),
    Newline,
}

/// Output of [`lex`]: the token stream with 1-based line numbers, plus a
/// flag per physical line saying whether it holds comment text.
type Lexed<'a> = (Vec<(Tok<'a>, usize)>, Vec<bool>);

#[expect(
    clippy::too_many_lines,
    reason = "single-pass byte scanner: one match over the token grammar keeps every lookahead rule beside its neighbours; a split would thread the cursor, line counter and token buffer through every helper"
)]
fn lex(src: &str) -> Result<Lexed<'_>, String> {
    let b = src.as_bytes();
    let n_lines = src.lines().count().max(1);
    let mut comment_lines = vec![false; n_lines];
    let mut toks = Vec::new();
    let mut i = if b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        0
    };
    let mut line = 1usize;

    while i < b.len() {
        let c = b[i];
        match c {
            b'\n' => {
                toks.push((Tok::Newline, line));
                line += 1;
                i += 1;
            }
            b' ' | b'\t' | b'\r' => i += 1,
            b'\\' if i + 1 < b.len() && b[i + 1] == b'\n' => {
                line += 1;
                i += 2;
            }
            b'\\' if i + 2 < b.len() && b[i + 1] == b'\r' && b[i + 2] == b'\n' => {
                line += 1;
                i += 3;
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'/' => {
                if line >= 1 && line <= n_lines {
                    comment_lines[line - 1] = true;
                }
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'*' => {
                if line >= 1 && line <= n_lines {
                    comment_lines[line - 1] = true;
                }
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    if b[i] == b'\n' {
                        line += 1;
                        if line <= n_lines {
                            comment_lines[line - 1] = true;
                        }
                    }
                    i += 1;
                }
                if i + 1 >= b.len() {
                    return Err(format!("line {line}: unterminated block comment"));
                }
                i += 2;
            }
            b'#' => {
                if line >= 1 && line <= n_lines {
                    comment_lines[line - 1] = true;
                }
                let rest = &src[i + 1..];
                let word: String = rest
                    .trim_start()
                    .chars()
                    .take_while(char::is_ascii_alphabetic)
                    .collect();
                match word.as_str() {
                    "if" | "ifdef" | "ifndef" => toks.push((Tok::Op("#if"), line)),
                    "else" | "elif" => toks.push((Tok::Op("#else"), line)),
                    "endif" => toks.push((Tok::Op("#endif"), line)),
                    _ => {}
                }
                while i < b.len() {
                    if b[i] == b'\\' && i + 1 < b.len() {
                        if b[i + 1] == b'\n' {
                            toks.push((Tok::Newline, line));
                            line += 1;
                        } else if b[i + 1] == b'\r' && i + 2 < b.len() && b[i + 2] == b'\n' {
                            toks.push((Tok::Newline, line));
                            line += 1;
                            i += 1;
                        }
                        i += 2;
                        continue;
                    }
                    if b[i] == b'\n' {
                        break;
                    }
                    i += 1;
                }
            }
            b'"' | b'\'' => {
                let (end, nl) = lex_quoted(b, i, line)?;
                line = nl;
                let tok = if c == b'"' {
                    Tok::Str(&src[i..end])
                } else {
                    Tok::Rune(&src[i..end])
                };
                toks.push((tok, line));
                i = end;
            }
            c if c.is_ascii_digit() => {
                let end = lex_number(b, i);
                toks.push((Tok::Number(&src[i..end]), line));
                i = end;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                let word = &src[start..i];
                let raw_next =
                    matches!(word, "R" | "uR" | "UR" | "LR" | "u8R") && i < b.len() && b[i] == b'"';
                if raw_next {
                    let (end, nl) = lex_raw_string(b, i, line)?;
                    line = nl;
                    toks.push((Tok::Str(&src[start..end]), line));
                    i = end;
                } else if ALL_KEYWORDS.binary_search(&word).is_ok() {
                    toks.push((Tok::Kw(word), line));
                } else {
                    toks.push((Tok::Ident(word), line));
                }
            }
            _ => {
                let op_len = halmi_lex::cfamily::punct_len(src, i);
                if op_len == 0 {
                    return Err(format!("line {line}: unexpected character `{}`", c as char));
                }
                toks.push((Tok::Op(&src[i..i + op_len]), line));
                i += op_len;
            }
        }
    }
    Ok((toks, comment_lines))
}

fn lex_quoted(b: &[u8], q: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = q;
    let mut line = line;
    while i < b.len() && matches!(b[i], b'u' | b'U' | b'L' | b'R' | b'8') {
        i += 1;
    }
    let raw = b[q..i].ends_with(b"R");
    if raw {
        i += 1;
        let dstart = i;
        while i < b.len() && b[i] != b'(' {
            i += 1;
        }
        if i >= b.len() {
            return Err(format!("line {line}: unterminated raw string"));
        }
        let delim = b[dstart..i].to_vec();
        let mut closer = Vec::with_capacity(delim.len() + 2);
        closer.push(b')');
        closer.extend_from_slice(&delim);
        closer.push(b'"');
        i += 1;
        while i < b.len() {
            if b[i] == b'\n' {
                line += 1;
            } else if b[i..].starts_with(&closer[..]) {
                return Ok((i + closer.len(), line));
            }
            i += 1;
        }
        return Err(format!("line {line}: unterminated raw string"));
    }
    let quote = b[i];
    i += 1;
    while i < b.len() {
        match b[i] {
            b'\\' => {
                i += 2;
                if i > 1 && i - 1 < b.len() && b[i - 1] == b'\n' {
                    line += 1;
                }
            }
            c if c == quote => return Ok((i + 1, line)),
            b'\n' => return Err(format!("line {line}: newline in string literal")),
            _ => i += 1,
        }
    }
    Err(format!("line {line}: unterminated string literal"))
}

const fn lex_number(b: &[u8], start: usize) -> usize {
    let mut i = start;
    if b[i] == b'0' && i + 1 < b.len() && matches!(b[i + 1], b'x' | b'X') {
        i += 2;
        while i < b.len() && (b[i].is_ascii_hexdigit() || b[i] == b'\'') {
            i += 1;
        }
    } else if b[i] == b'0' && i + 1 < b.len() && matches!(b[i + 1], b'b' | b'B') {
        i += 2;
        while i < b.len() && (b[i] == b'0' || b[i] == b'1' || b[i] == b'\'') {
            i += 1;
        }
    } else {
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'\'') {
            i += 1;
        }
        if i < b.len() && b[i] == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
            i += 1;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'\'') {
                i += 1;
            }
        }
        if i < b.len() && matches!(b[i], b'e' | b'E') {
            let mut j = i + 1;
            if j < b.len() && matches!(b[j], b'+' | b'-') {
                j += 1;
            }
            if j < b.len() && b[j].is_ascii_digit() {
                i = j;
                while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'\'') {
                    i += 1;
                }
            }
        }
    }
    while i < b.len() && b[i].is_ascii_alphanumeric() {
        i += 1;
    }
    i
}

fn skip_angles_fwd(toks: &[(Tok<'_>, usize)], open: usize) -> usize {
    let mut depth = 0usize;
    let mut k = open;
    while k < toks.len() {
        match &toks[k].0 {
            Tok::Op(o) if *o == "<" => depth += 1,
            Tok::Op(o) if *o == ">" => {
                depth -= 1;
                if depth == 0 {
                    return k + 1;
                }
            }
            Tok::Op(o) if *o == "<<" => {
                depth += 2;
            }
            Tok::Op(o) if *o == ">>" => {
                if depth <= 2 {
                    return k + 1;
                }
                depth -= 2;
            }
            _ => {}
        }
        k += 1;
    }
    0
}

fn typeish(t: &Tok<'_>) -> bool {
    match t {
        Tok::Ident(_) => true,
        Tok::Kw(k) => TYPEISH.contains(&&**k),
        Tok::Op(o) => matches!(&**o, "*" | "&" | "&&" | "::" | "~"),
        _ => false,
    }
}

fn skip_back_parens(toks: &[(Tok<'_>, usize)], close: usize) -> usize {
    let mut depth = 1usize;
    let mut j = close;
    while j > 0 {
        j -= 1;
        match &toks[j].0 {
            Tok::Op(o) if *o == ")" => depth += 1,
            Tok::Op(o) if *o == "(" => {
                depth -= 1;
                if depth == 0 {
                    return j;
                }
            }
            _ => {}
        }
    }
    0
}

fn skip_back_angles(toks: &[(Tok<'_>, usize)], close: usize) -> usize {
    let mut depth = match &toks[close].0 {
        Tok::Op(o) if *o == ">>" => 2,
        _ => 1,
    };
    let mut j = close;
    while j > 0 {
        j -= 1;
        match &toks[j].0 {
            Tok::Op(o) if *o == ">" => depth += 1,
            Tok::Op(o) if *o == ">>" => depth += 2,
            Tok::Op(o) if *o == "<" => {
                depth -= 1;
                if depth == 0 {
                    return j;
                }
            }
            _ => {}
        }
    }
    0
}

/// What the tokens before a `{` say about it: not a container body, or a
/// container body whose name may be anonymous (`enum { ... }`, `companion object`).
enum Container {
    Not,
    Body(Option<String>),
}

/// A `{` opens a container scope when the preceding tokens read
/// `class|struct|union|enum [class] NAME {` or `namespace [NAME] {`.
fn container_name_for(toks: &[(Tok<'_>, usize)], brace: usize) -> Container {
    let mut j = brace;
    if j == 0 {
        return Container::Not;
    }
    j -= 1;
    let mut name: Option<String> = None;
    if let Tok::Ident(n) = &toks[j].0 {
        name = Some((*n).to_string());
        if j == 0 {
            return Container::Not;
        }
        j -= 1;
    }
    match &toks[j].0 {
        Tok::Kw(k) if *k == "namespace" => Container::Body(name),
        Tok::Kw(k) if CONTAINER_KEYWORDS.contains(&&**k) => {
            if *k == "enum" && name.is_none() {
                return Container::Not;
            }
            Container::Body(name)
        }
        _ => Container::Not,
    }
}

struct Declarator {
    text: String,
    decl_name_start: usize,
    name_tok_end: usize,
    params_open: usize,
}

fn operator_declarator(toks: &[(Tok<'_>, usize)], i: usize) -> Option<(String, usize, usize)> {
    let mut declarator = "operator".to_string();
    let mut k = i + 1;
    while k < toks.len() {
        match &toks[k].0 {
            Tok::Op(o) if matches!(&**o, "(" | "[") => {
                let closer = if *o == "(" { ")" } else { "]" };
                if matches!(toks.get(k + 1), Some((Tok::Op(p), _)) if *p == closer) {
                    declarator.push_str(if *o == "(" { "()" } else { "[]" });
                    k += 2;
                    continue;
                }
                break;
            }
            Tok::Op(o) => {
                declarator.push_str(o);
                k += 1;
            }
            Tok::Ident(n) | Tok::Kw(n) => {
                if !declarator.ends_with(' ') {
                    declarator.push(' ');
                }
                declarator.push_str(n);
                k += 1;
            }
            _ => break,
        }
    }
    if k == i + 1 || !matches!(toks.get(k), Some((Tok::Op(o), _)) if *o == "(") {
        return None;
    }
    Some((declarator, k - 1, k))
}

fn declarator_at(toks: &[(Tok<'_>, usize)], i: usize) -> Option<Declarator> {
    let name = match &toks[i].0 {
        Tok::Ident(name) | Tok::Kw(name) => *name,
        _ => return None,
    };
    let mut j = i + 1;
    if matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "<") {
        let end = skip_angles_fwd(toks, j);
        if end == 0 || !matches!(toks.get(end), Some((Tok::Op(o), _)) if *o == "(") {
            return None;
        }
        j = end;
    }
    let mut text = name.to_string();
    let decl_name_start;
    let name_tok_end;
    if name == "operator" {
        let (operator, end, open) = operator_declarator(toks, i)?;
        text = operator;
        decl_name_start = i;
        name_tok_end = end;
        j = open;
    } else if matches!(toks.get(i.wrapping_sub(1)), Some((Tok::Op(t), _)) if *t == "~") {
        text = format!("~{text}");
        decl_name_start = i - 1;
        name_tok_end = i;
        j = i + 1;
        if matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "<") {
            let end = skip_angles_fwd(toks, j);
            if end == 0 || !matches!(toks.get(end), Some((Tok::Op(o), _)) if *o == "(") {
                return None;
            }
            j = end;
        }
    } else {
        let mut w = i;
        while w >= 2 {
            if matches!(&toks[w - 1].0, Tok::Op(o) if *o == "::")
                && matches!(&toks[w - 2].0, Tok::Ident(_))
            {
                let owner = match &toks[w - 2].0 {
                    Tok::Ident(n) => (*n).to_string(),
                    _ => String::new(),
                };
                text = format!("{owner}::{text}");
                w -= 2;
            } else {
                break;
            }
        }
        decl_name_start = w;
        name_tok_end = i;
    }
    Some(Declarator {
        text,
        decl_name_start,
        name_tok_end,
        params_open: j,
    })
}

fn parameters(toks: &[(Tok<'_>, usize)], open: usize) -> Option<(Vec<String>, usize)> {
    let mut j = open + 1;
    let mut params = Vec::new();
    let mut pending: Option<String> = None;
    let mut pdepth = 1usize;
    while j < toks.len() && pdepth > 0 {
        match &toks[j].0 {
            Tok::Op(o) if *o == "(" => pdepth += 1,
            Tok::Op(o) if *o == ")" => {
                pdepth -= 1;
                if pdepth == 0
                    && let Some(cand) = pending.take()
                    && !params.contains(&cand)
                {
                    params.push(cand);
                }
            }
            Tok::Ident(n) => pending = Some((*n).to_string()),
            Tok::Op(o) if *o == "," && pdepth == 1 => {
                if let Some(cand) = pending.take()
                    && !params.contains(&cand)
                {
                    params.push(cand);
                }
            }
            _ => {}
        }
        j += 1;
    }
    if pdepth != 0 {
        return None;
    }
    Some((params, j))
}

fn definition_gap(toks: &[(Tok<'_>, usize)], mut k: usize) -> Option<usize> {
    let mut defaulted = false;
    while k < toks.len() {
        match &toks[k].0 {
            Tok::Op(o) if *o == "{" => return Some(k),
            Tok::Op(o) if *o == ";" => {
                if defaulted {
                    return Some(k);
                }
                break;
            }
            Tok::Op(o) if *o == "=" => {
                let dflt = matches!(
                    toks.get(k + 1).map(|t| &t.0),
                    Some(Tok::Kw(p)) if matches!(&**p, "default" | "delete")
                );
                if !dflt {
                    break;
                }
                defaulted = true;
                k += 2;
            }
            Tok::Op(o) if *o == "(" || *o == "[" || *o == "<" => {
                let (closer, opener) = match *o {
                    "(" => (")", "("),
                    "[" => ("]", "["),
                    _ => (">", "<"),
                };
                let mut d = 1usize;
                k += 1;
                while k < toks.len() && d > 0 {
                    match &toks[k].0 {
                        Tok::Op(p) if *p == opener => d += 1,
                        Tok::Op(p) if *p == closer => d -= 1,
                        _ => {}
                    }
                    k += 1;
                }
            }
            Tok::Op(o)
                if *o == "->"
                    || *o == ","
                    || *o == "*"
                    || *o == "&"
                    || *o == "&&"
                    || *o == "::"
                    || *o == ":"
                    || *o == "~" =>
            {
                k += 1;
            }
            Tok::Newline | Tok::Ident(_) => k += 1,
            Tok::Kw(kw) => {
                if GAP_KEYWORDS.contains(&&**kw) || TYPEISH.contains(&&**kw) {
                    k += 1;
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    None
}

fn declaration_start(toks: &[(Tok<'_>, usize)], name_tok_end: usize) -> usize {
    let mut span_start = name_tok_end;
    let mut w = name_tok_end;
    let mut crossed_newline = false;
    while w > 0 {
        let prev = w - 1;
        match &toks[prev].0 {
            Tok::Op(o) if *o == ">" || *o == ">>" => {
                let open = skip_back_angles(toks, prev);
                if open > 0
                    && !matches!(&toks[open.wrapping_sub(1)].0, Tok::Kw(t) if *t == "template")
                {
                    span_start = open;
                    w = open;
                    crossed_newline = false;
                } else {
                    break;
                }
            }
            Tok::Op(o) if *o == ")" => {
                let open = skip_back_parens(toks, prev);
                let attr = open > 0
                    && matches!(
                        &toks[open - 1].0,
                        Tok::Ident(n) if matches!(&**n, "__attribute__" | "__declspec" | "__asm__" | "asm")
                    );
                if attr {
                    span_start = open - 1;
                    w = open - 1;
                    crossed_newline = false;
                } else {
                    break;
                }
            }
            Tok::Newline if !crossed_newline && w >= 2 && typeish_or_angle(toks, prev - 1) => {
                crossed_newline = true;
                span_start = prev - 1;
                w = prev - 1;
            }
            Tok::Newline => break,
            t if typeish(t) => {
                span_start = prev;
                w = prev;
            }
            _ => break,
        }
    }
    span_start
}

fn cpp_body_bounds(toks: &[(Tok<'_>, usize)], k: usize) -> (usize, usize, usize) {
    if matches!(toks.get(k), Some((Tok::Op(o), _)) if *o == ";") {
        return (k, k, toks[k].1);
    }
    let mut bd = 0usize;
    let mut cond_stack = Vec::new();
    let mut m = k;
    let mut el = toks[k].1;
    while m < toks.len() {
        match &toks[m].0 {
            Tok::Op(o) if *o == "{" => bd += 1,
            Tok::Op(o) if *o == "}" => {
                bd = bd.saturating_sub(1);
                if bd == 0 {
                    el = toks[m].1;
                    break;
                }
            }
            Tok::Op(o) if *o == "#if" => cond_stack.push(bd),
            Tok::Op(o) if *o == "#else" => bd = cond_stack.last().copied().unwrap_or(0),
            Tok::Op(o) if *o == "#endif" => {
                cond_stack.pop();
            }
            _ => {}
        }
        m += 1;
    }
    (k + 1, m, el)
}

fn analyze_cpp(src: &str) -> Result<Vec<RawFunction>, String> {
    let (toks, comment_lines) = lex(src)?;
    let mut collector = MetricsCollector::new(&CPP_SPEC, 0);
    let mut raws = Vec::new();
    let mut containers: Vec<(Option<String>, usize)> = Vec::new();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < toks.len() {
        match &toks[i].0 {
            Tok::Ident(name) | Tok::Kw(name)
                if (matches!(toks.get(i + 1), Some((Tok::Op(o), _)) if *o == "(" || *o == "<")
                    && !matches!(&toks[i].0, Tok::Kw(_)))
                    || (*name == "operator"
                        && matches!(&toks.get(i + 1).map(|t| &t.0), Some(Tok::Op(_)))) =>
            {
                if NON_FUNCTIONS.contains(name) {
                    i += 1;
                    continue;
                }
                let Some(declaration) = declarator_at(&toks, i) else {
                    i += 1;
                    continue;
                };
                let Some((mut params, j)) = parameters(&toks, declaration.params_open) else {
                    i += 1;
                    continue;
                };
                let Some(k) = definition_gap(&toks, j) else {
                    i += 1;
                    continue;
                };
                if params.len() == 1 && params[0] == "void" {
                    params.clear();
                }
                let span_start = declaration_start(&toks, declaration.name_tok_end);
                let decl_line = toks[span_start].1;
                let qualified = if declaration.text.contains("::") {
                    declaration.text.clone()
                } else {
                    match containers.iter().rev().find_map(|c| c.0.clone()) {
                        Some(owner) => format!("{owner}::{}", declaration.text),
                        None => declaration.text.clone(),
                    }
                };
                let ctor = declaration.text.ends_with(&format!("::{name}"))
                    || containers
                        .iter()
                        .rev()
                        .find_map(|c| c.0.clone())
                        .is_some_and(|owner| owner == *name && *name != "operator");
                let ret_span =
                    &toks[span_start.min(declaration.decl_name_start)..declaration.decl_name_start];
                let is_void =
                    ret_span.len() == 1 && matches!(&ret_span[0].0, Tok::Kw(v) if *v == "void");
                let has_result = !(ctor || is_void);
                let io_params = count_u32(params.len() + usize::from(has_result));
                let (body_start, body_end, end_line) = cpp_body_bounds(&toks, k);
                let raw = build_raw(
                    &mut collector,
                    &toks,
                    &comment_lines,
                    body_start,
                    body_end,
                    decl_line,
                    end_line,
                    &qualified,
                    io_params,
                );
                raws.push(raw);
                i = if body_end > i { body_end + 1 } else { i + 1 };
            }
            Tok::Op(o) if *o == "{" => {
                let container = container_name_for(&toks, i);
                depth += 1;
                if let Container::Body(c) = container {
                    containers.push((c, depth));
                }
                i += 1;
            }
            Tok::Op(o) if *o == "}" => {
                if let Some((_, pushed_at)) = containers.last()
                    && *pushed_at == depth
                {
                    containers.pop();
                }
                depth = depth.saturating_sub(1);
                i += 1;
            }
            _ => i += 1,
        }
    }
    Ok(raws)
}

fn typeish_or_angle(toks: &[(Tok<'_>, usize)], j: usize) -> bool {
    if typeish(&toks[j].0) {
        return true;
    }
    if matches!(&toks[j].0, Tok::Op(o) if *o == ">" || *o == ">>") {
        let open = skip_back_angles(toks, j);
        return open > 0
            && !matches!(&toks[open.wrapping_sub(1)].0, Tok::Kw(t) if *t == "template");
    }
    false
}

/// Raw string literal starting at the opening quote: `R"delim(...)delim"`.
fn lex_raw_string(b: &[u8], q: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = q + 1;
    let dstart = i;
    while i < b.len() && b[i] != b'(' {
        i += 1;
    }
    if i >= b.len() {
        return Err(format!("line {line}: unterminated raw string"));
    }
    let delim = b[dstart..i].to_vec();
    let mut closer = Vec::with_capacity(delim.len() + 2);
    closer.push(b')');
    closer.extend_from_slice(&delim);
    closer.push(b'"');
    i += 1;
    let mut line = line;
    while i < b.len() {
        if b[i] == b'\n' {
            line += 1;
        } else if b[i..].starts_with(&closer[..]) {
            return Ok((i + closer.len(), line));
        }
        i += 1;
    }
    Err(format!("line {line}: unterminated raw string"))
}

#[allow(clippy::too_many_arguments)]
fn build_raw<'a>(
    collector: &mut MetricsCollector<'a>,

    toks: &[(Tok<'a>, usize)],
    comment_lines: &[bool],
    body_start: usize,
    body_end: usize,
    def_line: usize,
    end_line: usize,
    name: &str,
    io_params: u32,
) -> RawFunction {
    for idx in body_start..body_end.min(toks.len()) {
        let (tok, ln) = &toks[idx];
        match tok {
            Tok::Newline => {
                collector.token(&halmi_lex::Tok::Newline, *ln);
            }
            Tok::Ident(n) => collector.token(&halmi_lex::Tok::Ident(n), *ln),
            Tok::Number(n) | Tok::Str(n) | Tok::Rune(n) => {
                collector.token(&halmi_lex::Tok::Str(n), *ln);
            }
            Tok::Kw(k) => collector.token(&halmi_lex::Tok::Keyword(k), *ln),
            Tok::Op(o) if o.starts_with('#') => {}
            Tok::Op(o) => match *o {
                "{" => collector.open_block(*ln),
                "}" => collector.close_block(*ln),
                "(" => {
                    let callee = toks.get(idx.wrapping_sub(1)).map(|t| &t.0);
                    let is_method = matches!(toks.get(idx.wrapping_sub(2)), Some((Tok::Op(p), _)) if *p == "." || *p == "->" || *p == "::");
                    let extends = matches!(toks.get(idx.wrapping_sub(3)), Some((Tok::Op(p), _)) if *p == ")" || *p == "]");
                    let is_call = match callee {
                        Some(Tok::Ident(_)) => true,
                        Some(Tok::Op(p)) => *p == ")",
                        _ => false,
                    };
                    if is_call {
                        collector.call(is_method, extends);
                    }
                    collector.token(&halmi_lex::Tok::Punct("("), *ln);
                }
                "[" => {
                    let callee = toks.get(idx.wrapping_sub(1)).map(|t| &t.0);
                    if matches!(callee, Some(Tok::Ident(_)))
                        || matches!(callee, Some(Tok::Op(p)) if *p == "]" || *p == ")")
                    {
                        collector.index();
                    } else {
                        collector.token(&halmi_lex::Tok::Punct("["), *ln);
                    }
                }
                _ => collector.token(&halmi_lex::Tok::Punct(o), *ln),
            },
        }
    }
    let outcome = collector.finish(io_params as usize);
    let span_lines = count_f64(end_line.saturating_sub(def_line) + 1);
    let comment_count = (def_line..=end_line)
        .filter(|l| comment_lines.get(l - 1).copied().unwrap_or(false))
        .count();
    RawFunction {
        path: String::new(),
        language: Language::Cpp,
        name: name.to_string(),
        start_line: count_u32(def_line),
        end_line: count_u32(end_line),
        counts: outcome.counts,
        cyclomatic: outcome.cyclomatic,
        loc: outcome.loc,
        nesting: outcome.nesting,
        comment_density: Some(count_f64(comment_count) / span_lines.max(1.0)),
        method_chain: outcome.method_chain,
    }
}

#[cfg(test)]
mod kw_tests {
    #[test]
    fn keywords_sorted_for_binary_search() {
        assert!(super::ALL_KEYWORDS.windows(2).all(|w| w[0] <= w[1]));
    }
}
