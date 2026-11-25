//! halmi-cs: C# language adapter.
//!
//! Hand lexer + declaration scanner on [`halmi_lex`] shared machinery
//! (halmi-java shape with C# declarations). Zero dependencies, no
//! tree-sitter: the tree-sitter-c_sharp oracle is a test-side
//! differential only.
//!
//! * Definitions: `method_declaration` / `constructor_declaration`
//!   parity: methods, constructors, operators (`operator +`),
//!   expression-bodied members (`=> expr;`), and local functions
//!   count. Attribute runs (`[Obsolete("x")]`) fold into the span.
//!   Properties, accessors, indexers, destructors, enum members, and
//!   lambdas do not count.
//! * Candidates are depth-gated to container member level
//!   (class/struct/interface/record/enum and file-scoped
//!   `namespace X;`), rejecting calls inside bodies.
//! * `decl_op`: the declarator `(` (C# has no `fn` keyword).

#![deny(unsafe_code)]

use halmi_core::adapter::{LanguageAdapter, SourceAnalysis};
use halmi_core::report::{Language, RawFunction, count_f64, count_u32};
use halmi_lex::{LangSpec, LocMode, MetricsCollector};

#[derive(Debug, Clone, Copy, Default)]
pub struct CsAdapter;

impl LanguageAdapter for CsAdapter {
    fn language(&self) -> &'static str {
        "Cs"
    }

    fn analyze(&self, path: &str, src: &str) -> SourceAnalysis {
        match analyze_cs(src) {
            Ok(mut raws) => {
                for raw in &mut raws {
                    raw.path = path.to_string();
                    raw.language = Language::Cs;
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

const CS_SPEC: LangSpec = LangSpec {
    decl_op: "(",
    keyword_operators: CONTROL_KEYWORDS,
    cc_keywords: &["if", "for", "foreach", "while", "case", "catch"],
    separator_ops: &["(", ")", "[", "]", "{", "}", ",", ";", ":"],
    cc_ops: &["&&", "||"],
    loc_mode: LocMode::Physical,
};

const CONTROL_KEYWORDS: &[&str] = &[
    "if",
    "else",
    "for",
    "foreach",
    "while",
    "do",
    "switch",
    "case",
    "default",
    "break",
    "continue",
    "return",
    "throw",
    "try",
    "catch",
    "finally",
    "new",
    "typeof",
    "nameof",
    "sizeof",
    "lock",
    "using",
    "yield",
    "await",
    "select",
    "where",
    "orderby",
    "from",
    "in",
    "operator",
    "checked",
    "unchecked",
];

const ALL_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "async",
    "await",
    "base",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "checked",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "double",
    "dynamic",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "float",
    "for",
    "foreach",
    "get",
    "goto",
    "if",
    "implicit",
    "in",
    "init",
    "int",
    "interface",
    "internal",
    "is",
    "lock",
    "long",
    "nameof",
    "namespace",
    "new",
    "not",
    "null",
    "object",
    "on",
    "operator",
    "or",
    "out",
    "override",
    "params",
    "partial",
    "private",
    "protected",
    "public",
    "readonly",
    "record",
    "ref",
    "return",
    "sbyte",
    "sealed",
    "select",
    "set",
    "short",
    "sizeof",
    "stackalloc",
    "static",
    "string",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "uint",
    "ulong",
    "unchecked",
    "unsafe",
    "ushort",
    "using",
    "var",
    "virtual",
    "void",
    "volatile",
    "when",
    "where",
    "while",
    "with",
    "yield",
];

const MODIFIERS: &[&str] = &[
    "public",
    "private",
    "protected",
    "internal",
    "static",
    "readonly",
    "abstract",
    "sealed",
    "virtual",
    "override",
    "async",
    "extern",
    "unsafe",
    "partial",
    "new",
    "required",
];

const CONTAINER_KEYWORDS: &[&str] = &["class", "interface", "struct", "record", "enum"];

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
            b'"' if src[i..].starts_with("\"\"\"") => {
                let (end, nl) = lex_raw_string(b, i, line)?;
                line = nl;
                toks.push((Tok::Str(&src[i..end]), line));
                i = end;
            }
            b'$' if src[i..].starts_with("$\"") || src[i..].starts_with("$@\"") => {
                let (end, nl) = lex_interpolated(b, i, line)?;
                line = nl;
                toks.push((Tok::Str(&src[i..end]), line));
                i = end;
            }
            b'@' if i + 1 < b.len() && b[i + 1] == b'"' => {
                let (end, nl) = lex_verbatim(b, i + 1, line)?;
                line = nl;
                toks.push((Tok::Str(&src[i..end]), line));
                i = end;
            }
            b'"' => {
                let (end, nl) = lex_string(b, i, line)?;
                line = nl;
                toks.push((Tok::Str(&src[i..end]), line));
                i = end;
            }
            b'\'' => {
                let (end, nl) = lex_char(b, i, line)?;
                line = nl;
                toks.push((Tok::Rune(&src[i..end]), line));
                i = end;
            }
            c if c.is_ascii_digit() => {
                let end = lex_number(b, i);
                toks.push((Tok::Number(&src[i..end]), line));
                i = end;
            }
            c if c.is_ascii_alphabetic() || c == b'_' || c == b'@' => {
                let start = i;
                if b[i] == b'@' {
                    i += 1;
                }
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                let word = &src[start..i];
                let bare = word.trim_start_matches('@');
                if ALL_KEYWORDS.binary_search(&bare).is_ok() {
                    toks.push((Tok::Kw(bare), line));
                } else {
                    toks.push((Tok::Ident(bare), line));
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

fn lex_raw_string(b: &[u8], q: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = q + 3;
    let mut line = line;
    while i + 2 < b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if b[i] == b'"' && b[i + 1] == b'"' && b[i + 2] == b'"' {
            return Ok((i + 3, line));
        }
        i += 1;
    }
    Err(format!("line {line}: unterminated raw string"))
}

fn lex_interpolated(b: &[u8], q: usize, line: usize) -> Result<(usize, usize), String> {
    let mut line = line;
    let mut i = q + 1;
    if i < b.len() && b[i] == b'@' {
        i += 1;
    }
    if i >= b.len() || b[i] != b'"' {
        return Err(format!("line {line}: bad interpolated string"));
    }
    i += 1;
    let verbatim = b[q + 1] == b'@';
    while i < b.len() {
        if b[i] == b'{' && i + 1 < b.len() && b[i + 1] == b'{' {
            i += 2;
            continue;
        }
        if b[i] == b'"' {
            if verbatim {
                if i + 1 < b.len() && b[i + 1] == b'"' {
                    i += 2;
                    continue;
                }
                return Ok((i + 1, line));
            }
            return Ok((i + 1, line));
        }
        if b[i] == b'\\' && !verbatim {
            i += 2;
            continue;
        }
        if b[i] == b'\n' {
            if !verbatim {
                return Err(format!("line {line}: newline in interpolated string"));
            }
            line += 1;
        }
        i += 1;
    }
    Err(format!("line {line}: unterminated interpolated string"))
}

fn lex_verbatim(b: &[u8], q: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = q + 1;
    let mut line = line;
    while i < b.len() {
        if b[i] == b'"' {
            if i + 1 < b.len() && b[i + 1] == b'"' {
                i += 2;
                continue;
            }
            return Ok((i + 1, line));
        }
        if b[i] == b'\n' {
            line += 1;
        }
        i += 1;
    }
    Err(format!("line {line}: unterminated verbatim string"))
}

fn lex_string(b: &[u8], q: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = q + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Ok((i + 1, line)),
            b'\n' => return Err(format!("line {line}: newline in string")),
            _ => i += 1,
        }
    }
    Err(format!("line {line}: unterminated string"))
}

fn lex_char(b: &[u8], q: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = q + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'\'' => return Ok((i + 1, line)),
            b'\n' => return Err(format!("line {line}: newline in char literal")),
            _ => i += 1,
        }
    }
    Err(format!("line {line}: unterminated char literal"))
}

const fn lex_number(b: &[u8], start: usize) -> usize {
    let mut i = start;
    if b[i] == b'0' && i + 1 < b.len() && matches!(b[i + 1], b'x' | b'X') {
        i += 2;
        while i < b.len() && (b[i].is_ascii_hexdigit() || b[i] == b'_') {
            i += 1;
        }
    } else if b[i] == b'0' && i + 1 < b.len() && matches!(b[i + 1], b'b' | b'B') {
        i += 2;
        while i < b.len() && matches!(b[i], b'0' | b'1' | b'_') {
            i += 1;
        }
    } else {
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
            i += 1;
        }
        if i < b.len() && b[i] == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
            i += 1;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
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
                while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
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
        Tok::Op(o) if *o == ">>>" => 3,
        _ => 1,
    };
    let mut j = close;
    while j > 0 {
        j -= 1;
        match &toks[j].0 {
            Tok::Op(o) if *o == ">" => depth += 1,
            Tok::Op(o) if *o == ">>" => depth += 2,
            Tok::Op(o) if *o == ">>>" => depth += 3,
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

fn typeish(t: &Tok<'_>) -> bool {
    match t {
        Tok::Ident(_) => true,
        Tok::Kw(k) => {
            MODIFIERS.contains(&&**k)
                || matches!(
                    &**k,
                    "int"
                        | "long"
                        | "short"
                        | "sbyte"
                        | "byte"
                        | "char"
                        | "float"
                        | "double"
                        | "decimal"
                        | "bool"
                        | "string"
                        | "object"
                        | "void"
                        | "var"
                        | "dynamic"
                        | "event"
                        | "delegate"
                        | "partial"
                        | "async"
                )
        }
        Tok::Op(o) => matches!(&**o, "*" | "?" | "::" | "[" | "]" | "<" | ">"),
        _ => false,
    }
}

/// What the tokens before a `{` say about it: not a container body, or a
/// container body whose name may be anonymous (`enum { ... }`, `companion object`).
enum Container {
    Not,
    Body(Option<String>),
}

/// A `{` opens a container scope when the preceding tokens read
/// `[attrs] modifiers class|struct|interface|record|enum NAME [<...>]
/// [bases/where...] {`, or `) {` of an anonymous-object creation.
fn container_name_for(toks: &[(Tok<'_>, usize)], brace: usize) -> Container {
    let mut j = brace;
    if j == 0 {
        return Container::Not;
    }
    j -= 1;
    if matches!(&toks[j].0, Tok::Op(o) if *o == ">" || *o == ">>") {
        let open = skip_back_angles(toks, j);
        if open == 0 || open > j {
            return Container::Not;
        }
        j = open - 1;
    }
    if matches!(&toks[j].0, Tok::Op(o) if *o == ")") {
        let open = skip_back_parens(toks, j);
        if open == 0 || open > j {
            return Container::Not;
        }
        j = open.saturating_sub(1);
    }
    let mut name: Option<String> = None;
    if let Tok::Ident(n) = &toks[j].0 {
        name = Some((*n).to_string());
        if j == 0 {
            return Container::Not;
        }
        j -= 1;
        if matches!(&toks[j].0, Tok::Op(o) if *o == ",") {
            return Container::Body(name);
        }
    }
    let name = name;
    loop {
        if j == 0 {
            break;
        }
        match &toks[j].0 {
            Tok::Newline | Tok::Ident(_) => j -= 1,
            Tok::Kw(k) if matches!(&**k, "where" | "in") => j -= 1,
            Tok::Op(o) if matches!(&**o, "," | "." | ":" | "?") => j -= 1,
            Tok::Op(o) if *o == ">" || *o == ">>" => {
                let open = skip_back_angles(toks, j);
                if open == 0 || open > j {
                    break;
                }
                j = open - 1;
            }
            Tok::Op(o) if *o == ")" => {
                let open = skip_back_parens(toks, j);
                if open == 0 || open > j {
                    break;
                }
                j = open - 1;
            }
            _ => break,
        }
    }
    match &toks[j].0 {
        Tok::Kw(k) if CONTAINER_KEYWORDS.contains(&&**k) => {
            let real_name = match toks.get(j + 1).map(|t| &t.0) {
                Some(Tok::Ident(n)) => Some((*n).to_string()),
                _ => name,
            };
            Container::Body(real_name)
        }
        _ => Container::Not,
    }
}

fn method_parameters(toks: &[(Tok<'_>, usize)], open: usize) -> Option<(Vec<String>, usize)> {
    let mut params = Vec::new();
    let mut pending: Option<String> = None;
    let mut pdepth = 1usize;
    let mut j = open + 1;
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

fn method_terminator(toks: &[(Tok<'_>, usize)], mut k: usize) -> Option<(usize, bool)> {
    let mut expr_body = false;
    while k < toks.len() {
        match &toks[k].0 {
            Tok::Op(o) if *o == "{" && !expr_body => return Some((k, true)),
            Tok::Op(o) if *o == "{" && expr_body => {
                let mut d = 1usize;
                k += 1;
                while k < toks.len() && d > 0 {
                    match &toks[k].0 {
                        Tok::Op(p) if *p == "{" => d += 1,
                        Tok::Op(p) if *p == "}" => d -= 1,
                        _ => {}
                    }
                    k += 1;
                }
            }
            Tok::Op(o) if *o == ";" => return Some((k, false)),
            Tok::Op(o) if *o == "=>" => {
                expr_body = true;
                k += 1;
            }
            Tok::Op(_) | Tok::Number(_) | Tok::Str(_) | Tok::Rune(_) if expr_body => k += 1,
            Tok::Op(o) if *o == "(" || *o == "[" => {
                let (closer, opener) = if *o == "(" { (")", "(") } else { ("]", "[") };
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
            Tok::Newline | Tok::Ident(_) => k += 1,
            Tok::Kw(kw) => {
                if typeish(&Tok::Kw(kw)) {
                    k += 1;
                } else {
                    break;
                }
            }
            Tok::Op(o) if matches!(&**o, "." | "," | "*" | "?" | "::" | "<" | ">" | "??") => {
                k += 1;
            }
            _ => break,
        }
    }
    None
}

fn declaration_start(toks: &[(Tok<'_>, usize)], i: usize) -> usize {
    let mut span_start = i;
    let mut w = i;
    let mut crossed_newline = false;
    while w > 0 {
        let prev = w - 1;
        match &toks[prev].0 {
            Tok::Op(o) if *o == "]" => {
                let open = bracket_group_open(toks, prev);
                let attr = open > 1
                    && matches!(&toks[open - 1].0, Tok::Ident(n) if is_attribute_context(toks, open - 1));
                if attr {
                    span_start = open - 1;
                    w = open - 1;
                    crossed_newline = false;
                } else if open > 0 {
                    span_start = open;
                    w = open;
                    crossed_newline = false;
                } else {
                    break;
                }
            }
            Tok::Op(o) if *o == ">" || *o == ">>" => {
                let open = skip_back_angles(toks, prev);
                if open > 0 && open <= prev {
                    span_start = open;
                    w = open;
                    crossed_newline = false;
                } else {
                    break;
                }
            }
            Tok::Op(o) if *o == ")" => {
                let open = skip_back_parens(toks, prev);
                let attr = open > 1
                    && matches!(&toks[open - 1].0, Tok::Ident(n) if is_attribute_context(toks, open - 1));
                if attr {
                    span_start = open - 1;
                    w = open - 1;
                    crossed_newline = false;
                } else {
                    break;
                }
            }
            Tok::Newline
                if !crossed_newline
                    && w >= 2
                    && (typeish(&toks[prev - 1].0)
                        || matches!(&toks[prev - 1].0, Tok::Op(p) if *p == "]")) =>
            {
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

fn cs_body_bounds(toks: &[(Tok<'_>, usize)], k: usize, brace: bool) -> (usize, usize, usize) {
    if !brace {
        return (k, k, toks[k].1);
    }
    let mut bd = 0usize;
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
            _ => {}
        }
        m += 1;
    }
    (k + 1, m, el)
}

#[expect(
    clippy::too_many_lines,
    reason = "one pass over the token stream that tracks brace depth, the container stack and the open function span together; a split would share about ten mutable state variables through every helper"
)]
fn analyze_cs(src: &str) -> Result<Vec<RawFunction>, String> {
    let (toks, comment_lines) = lex(src)?;
    let mut collector = MetricsCollector::new(&CS_SPEC, 0);
    let mut raws = Vec::new();
    let mut containers: Vec<(Option<String>, usize, bool)> = Vec::new();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < toks.len() {
        match &toks[i].0 {
            Tok::Ident(name)
                if matches!(toks.get(i + 1), Some((Tok::Op(o), _)) if *o == "(")
                    && containers.last().is_some_and(|(_, _, done)| *done)
                    && (containers.last().is_some_and(|(_, d, _)| *d == depth)
                        || (containers.last().is_some_and(|(_, d, _)| *d + 1 == depth)
                            && toks.get(i.wrapping_sub(1)).is_some_and(|t| typeish(&t.0))
                            && !matches!(
                                toks.get(i.wrapping_sub(1)),
                                Some((Tok::Kw(p), _)) if *p == "new" || *p == "return" || *p == "throw"
                            )))
                    && !matches!(toks.get(i.wrapping_sub(1)), Some((Tok::Op(o), _)) if *o == "." || *o == "::" || *o == "new" || *o == "=>") =>
            {
                if *name == "operator" {
                    i += 1;
                    continue;
                }
                let name = (*name).to_string();
                let owner = containers
                    .iter()
                    .filter_map(|(n, _, _)| n.clone())
                    .collect::<Vec<_>>()
                    .join("::");
                let is_ctor = owner == name;
                let qualified = if owner.is_empty() {
                    name
                } else {
                    format!("{owner}::{name}")
                };
                let Some((params, j)) = method_parameters(&toks, i + 1) else {
                    i += 1;
                    continue;
                };
                let Some((k, terminator_brace)) = method_terminator(&toks, j) else {
                    i += 1;
                    continue;
                };
                let span_start = declaration_start(&toks, i);
                let decl_line = toks[span_start].1;
                let mut ret_words: Vec<&str> = Vec::new();
                for t in &toks[span_start..i] {
                    match &t.0 {
                        Tok::Ident(n) | Tok::Kw(n) => ret_words.push(n),
                        _ => {}
                    }
                }
                let is_void = ret_words.len() == 1 && ret_words[0] == "void";
                let has_result = !(is_ctor || is_void);
                let io_params = count_u32(params.len() + usize::from(has_result));
                let (body_start, body_end, end_line) = cs_body_bounds(&toks, k, terminator_brace);
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
                if terminator_brace && body_start > i {
                    depth += 1;
                    i = body_start;
                } else {
                    i += 1;
                }
            }
            Tok::Kw(k)
                if *k == "namespace"
                    && matches!(&toks.get(i + 1).map(|t| &t.0), Some(Tok::Ident(_)))
                    && matches!(&toks.get(i + 2).map(|t| &t.0), Some(Tok::Op(o)) if *o == ";") =>
            {
                i += 3;
            }
            Tok::Op(o) if *o == "{" => {
                let container = container_name_for(&toks, i);
                depth += 1;
                if let Container::Body(c) = container {
                    containers.push((c, depth, true));
                }
                i += 1;
            }
            Tok::Op(o) if *o == "}" => {
                if let Some((_, pushed_at, _)) = containers.last()
                    && *pushed_at == depth
                {
                    containers.pop();
                }
                depth = depth.saturating_sub(1);
                i += 1;
            }
            Tok::Op(o) if *o == ";" => {
                if let Some((_, d, done)) = containers.last_mut()
                    && *d == depth
                    && !*done
                {
                    *done = true;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    Ok(raws)
}

fn is_attribute_context(toks: &[(Tok<'_>, usize)], j: usize) -> bool {
    matches!(&toks[j].0, Tok::Ident(n) if matches!(&**n, "Obsolete" | "STAThread" | "Attribute" | "TestMethod" | "Fact"))
        || matches!(toks.get(j.wrapping_sub(1)).map(|t| &t.0), Some(Tok::Op(p)) if *p == "]" || *p == ",")
        || matches!(toks.get(j.wrapping_sub(1)).map(|t| &t.0), Some(Tok::Op(p)) if *p == "[")
}

fn bracket_group_open(toks: &[(Tok<'_>, usize)], close: usize) -> usize {
    let mut depth = 1usize;
    let mut j = close;
    while j > 0 {
        j -= 1;
        match &toks[j].0 {
            Tok::Op(o) if *o == "]" => depth += 1,
            Tok::Op(o) if *o == "[" => {
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
            Tok::Op(o) => match *o {
                "{" => collector.open_block(*ln),
                "}" => collector.close_block(*ln),
                "(" => {
                    let callee = toks.get(idx.wrapping_sub(1)).map(|t| &t.0);
                    let is_method = matches!(toks.get(idx.wrapping_sub(2)), Some((Tok::Op(p), _)) if *p == "." || *p == "::");
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
        language: Language::Cs,
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
