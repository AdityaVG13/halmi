//! halmi-kotlin: Kotlin language adapter.
//!
//! Hand lexer + declaration scanner on [`halmi_lex`] shared machinery.
//! Zero dependencies, no tree-sitter: the tree-sitter-kotlin oracle is
//! a test-side differential only.
//!
//! * Definitions: `function_declaration` / `secondary_constructor`
//!   parity: every `fun` (top-level, in classes/objects/interfaces/
//!   enums, companion objects, with receiver types like
//!   `fun String.shout()`, generic `<reified T>`, expression bodies
//!   `= expr`, suspend/inline/operator modifiers) and
//!   `constructor() : this(...)` secondary constructors. Primary
//!   constructors, `init` blocks, and lambdas do not count.
//! * Annotation and modifier runs fold into the span.
//! * Qualification: container chain (`Widget::area`); top-level
//!   functions are unqualified.
//! * `decl_op`: `fun`.

#![deny(unsafe_code)]

use halmi_core::adapter::{LanguageAdapter, SourceAnalysis};
use halmi_core::report::{Language, RawFunction, count_f64, count_u32};
use halmi_lex::{LangSpec, LocMode, MetricsCollector};

#[derive(Debug, Clone, Copy, Default)]
pub struct KotlinAdapter;

impl LanguageAdapter for KotlinAdapter {
    fn language(&self) -> &'static str {
        "Kotlin"
    }

    fn analyze(&self, path: &str, src: &str) -> SourceAnalysis {
        match analyze_kotlin(src) {
            Ok(mut raws) => {
                for raw in &mut raws {
                    raw.path = path.to_string();
                    raw.language = Language::Kotlin;
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

const KT_SPEC: LangSpec = LangSpec {
    decl_op: "fun",
    keyword_operators: CONTROL_KEYWORDS,
    cc_keywords: &["if", "for", "while", "when", "catch"],
    separator_ops: &["(", ")", "[", "]", "{", "}", ",", ";", ":"],
    cc_ops: &["&&", "||"],
    loc_mode: LocMode::Physical,
};

const CONTROL_KEYWORDS: &[&str] = &[
    "if", "else", "for", "while", "do", "when", "break", "continue", "return", "throw", "try",
    "catch", "finally", "is", "in", "as", "fun",
];

const ALL_KEYWORDS: &[&str] = &[
    "as",
    "break",
    "by",
    "catch",
    "class",
    "companion",
    "const",
    "constructor",
    "continue",
    "crossinline",
    "data",
    "do",
    "dynamic",
    "else",
    "enum",
    "expect",
    "external",
    "false",
    "final",
    "finally",
    "for",
    "fun",
    "get",
    "if",
    "import",
    "in",
    "infix",
    "init",
    "inline",
    "inner",
    "interface",
    "internal",
    "is",
    "lateinit",
    "noinline",
    "null",
    "object",
    "open",
    "operator",
    "out",
    "override",
    "package",
    "private",
    "protected",
    "public",
    "reified",
    "return",
    "sealed",
    "set",
    "super",
    "suspend",
    "tailrec",
    "this",
    "throw",
    "true",
    "try",
    "typealias",
    "typeof",
    "val",
    "var",
    "vararg",
    "when",
    "where",
    "while",
];

const MODIFIERS: &[&str] = &[
    "private",
    "internal",
    "protected",
    "public",
    "open",
    "final",
    "abstract",
    "override",
    "lateinit",
    "const",
    "suspend",
    "inline",
    "noinline",
    "crossinline",
    "tailrec",
    "operator",
    "infix",
    "external",
    "expect",
    "inner",
    "companion",
    "data",
    "sealed",
    "annotation",
];

const CONTAINER_KEYWORDS: &[&str] = &["class", "interface", "object", "enum"];

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
                let mut comment_depth = 1usize;
                while i + 1 < b.len() && comment_depth > 0 {
                    if b[i] == b'/' && b[i + 1] == b'*' {
                        comment_depth += 1;
                        i += 2;
                    } else if b[i] == b'*' && b[i + 1] == b'/' {
                        comment_depth -= 1;
                        i += 2;
                    } else {
                        if b[i] == b'\n' {
                            line += 1;
                            if line <= n_lines {
                                comment_lines[line - 1] = true;
                            }
                        }
                        i += 1;
                    }
                }
                if comment_depth != 0 {
                    return Err(format!("line {line}: unterminated block comment"));
                }
            }
            b'"' if src[i..].starts_with("\"\"\"") => {
                let (end, nl) = lex_raw(b, i, line)?;
                line = nl;
                toks.push((Tok::Str(&src[i..end]), line));
                i = end;
            }
            b'"' => {
                let (end, nl) = lex_template(b, i, line)?;
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
            b'`' => {
                let start = i;
                i += 1;
                while i < b.len() && b[i] != b'`' && b[i] != b'\n' {
                    i += 1;
                }
                if i >= b.len() || b[i] == b'\n' {
                    return Err(format!("line {line}: unterminated backtick identifier"));
                }
                i += 1;
                toks.push((Tok::Ident(&src[start + 1..i - 1]), line));
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
                if ALL_KEYWORDS.binary_search(&word).is_ok() {
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

fn lex_raw(b: &[u8], q: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = q + 3;
    let mut line = line;
    while i + 2 < b.len() {
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

/// Regular string with `$name` / `${expr}` template interpolations
/// (both nested-string aware); the whole template is one token.
fn lex_template(b: &[u8], q: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = q + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Ok((i + 1, line)),
            b'\n' => return Err(format!("line {line}: newline in string")),
            b'$' if i + 1 < b.len() && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_') => {
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
            }
            b'$' if i + 1 < b.len() && b[i + 1] == b'{' => {
                let mut d = 0usize;
                while i < b.len() {
                    match b[i] {
                        b'{' => d += 1,
                        b'}' => {
                            d -= 1;
                            if d == 0 {
                                break;
                            }
                        }
                        b'"' => {
                            i += 1;
                            while i < b.len() && b[i] != b'"' {
                                if b[i] == b'\\' {
                                    i += 1;
                                }
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
            }
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
    while i < b.len() && b[i].is_ascii_alphabetic() {
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

/// What the tokens before a `{` say about it: not a container body, or a
/// container body whose name may be anonymous (`enum { ... }`, `companion object`).
enum Container {
    Not,
    Body(Option<String>),
}

/// A `{` opens a container scope when the preceding tokens read
/// `NAME (<primary ctor>)? : supertypes {` after a class/object/
/// interface/enum keyword (skipping annotations and modifiers).
fn container_name_for(toks: &[(Tok<'_>, usize)], brace: usize) -> Container {
    let mut j = brace;
    if j == 0 {
        return Container::Not;
    }
    j -= 1;
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
    }
    loop {
        if j == 0 {
            break;
        }
        match &toks[j].0 {
            Tok::Newline | Tok::Ident(_) => j -= 1,
            Tok::Kw(k)
                if matches!(
                    &**k,
                    "where"
                        | "by"
                        | "in"
                        | "companion"
                        | "data"
                        | "inner"
                        | "sealed"
                        | "annotation"
                        | "open"
                        | "final"
                        | "abstract"
                        | "internal"
                        | "private"
                        | "protected"
                        | "public"
                        | "external"
                        | "expect"
                        | "inline"
                        | "value"
                        | "fun"
                        | "suspend"
                ) =>
            {
                j -= 1;
            }
            Tok::Op(o) if matches!(&**o, "," | "." | ":" | "?" | "<" | ">" | ">>") => j -= 1,
            Tok::Op(o) if *o == ")" => {
                let open = skip_back_parens(toks, j);
                if open == 0 || open > j {
                    break;
                }
                j = open.saturating_sub(1);
            }
            Tok::Op(o) if *o == "@" => return Container::Body(None),
            _ => break,
        }
    }
    match &toks[j].0 {
        Tok::Kw(k) if CONTAINER_KEYWORDS.contains(&&**k) => {
            if *k == "object"
                && matches!(&toks.get(j.wrapping_sub(1)).map(|t| &t.0), Some(Tok::Kw(p)) if *p == "companion")
            {
                return Container::Body(Some("Companion".to_string()));
            }
            let real_name = match toks.get(j + 1).map(|t| &t.0) {
                Some(Tok::Ident(n)) => Some((*n).to_string()),
                _ => name,
            };
            Container::Body(real_name)
        }
        _ => Container::Not,
    }
}

fn function_head(toks: &[(Tok<'_>, usize)], i: usize) -> Option<(String, Vec<String>, usize)> {
    let mut j = i + 1;
    if matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "<") {
        let end = skip_angles_fwd(toks, j);
        if end == 0 {
            return None;
        }
        j = end;
    }
    let mut name = match toks.get(j).map(|t| &t.0) {
        Some(Tok::Ident(n) | Tok::Kw(n)) => (*n).to_string(),
        _ => return None,
    };
    j += 1;
    while matches!(
        toks.get(j),
        Some((Tok::Op(o), _)) if *o == "." || *o == "?." || *o == "::"
    ) {
        if let Some((Tok::Ident(n), _)) = toks.get(j + 1) {
            name = (*n).to_string();
            j += 2;
        } else {
            break;
        }
    }
    if !matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "(") {
        return None;
    }
    let mut params = Vec::new();
    let mut pending: Option<String> = None;
    let mut pdepth = 1usize;
    j += 1;
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
    Some((name, params, j))
}

fn function_terminator(toks: &[(Tok<'_>, usize)], mut k: usize) -> Option<usize> {
    let mut expr_body = false;
    while k < toks.len() {
        match &toks[k].0 {
            Tok::Op(o) if *o == "{" && !expr_body => return Some(k),
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
            Tok::Op(o) if *o == "=>" || *o == "=" => {
                expr_body = true;
                k += 1;
            }
            Tok::Op(o) if *o == ";" && expr_body => return Some(k),
            Tok::Newline if expr_body => {
                let mut peek = k + 1;
                while matches!(toks.get(peek).map(|t| &t.0), Some(Tok::Newline)) {
                    peek += 1;
                }
                let starts_declaration = match toks.get(peek).map(|t| &t.0) {
                    Some(Tok::Kw(p)) => {
                        matches!(
                            &**p,
                            "fun"
                                | "val"
                                | "var"
                                | "class"
                                | "object"
                                | "interface"
                                | "enum"
                                | "companion"
                                | "init"
                                | "constructor"
                                | "package"
                                | "import"
                                | "typealias"
                        ) || MODIFIERS.contains(&&**p)
                    }
                    Some(Tok::Op(p)) => matches!(&**p, "}" | "@"),
                    None => true,
                    _ => false,
                };
                let literal_then_ident =
                    matches!(
                        toks.get(k.wrapping_sub(1)).map(|t| &t.0),
                        Some(Tok::Str(_) | Tok::Number(_) | Tok::Rune(_))
                    ) && matches!(toks.get(peek).map(|t| &t.0), Some(Tok::Ident(_)));
                if starts_declaration || literal_then_ident {
                    return Some(k);
                }
                k += 1;
            }
            Tok::Newline => return Some(k),
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
            _ => k += 1,
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
            Tok::Op(o) if *o == ")" => {
                let open = skip_back_parens(toks, prev);
                let attr =
                    open > 1 && matches!(&toks[open.wrapping_sub(2)].0, Tok::Op(p) if *p == "@");
                if attr {
                    span_start = open - 2;
                    w = open - 2;
                    crossed_newline = false;
                } else {
                    break;
                }
            }
            Tok::Newline
                if !crossed_newline
                    && w >= 2
                    && (matches!(&toks[prev - 1].0, Tok::Kw(p) if MODIFIERS.contains(&&**p))
                        || matches!(&toks[prev - 1].0, Tok::Op(p) if *p == "@")
                        || (matches!(&toks[prev - 1].0, Tok::Ident(_))
                            && matches!(&toks[prev - 2].0, Tok::Op(p) if *p == "@")
                            && !matches!(&toks[prev].0, Tok::Op(p) if *p == "("))
                        || (matches!(&toks[prev - 1].0, Tok::Op(p) if *p == ")") && {
                            let open = skip_back_parens(toks, prev - 1);
                            open > 1
                                && matches!(&toks[open - 1].0, Tok::Ident(_))
                                && matches!(&toks[open.wrapping_sub(2)].0, Tok::Op(q) if *q == "@")
                        })) =>
            {
                crossed_newline = true;
                if matches!(&toks[prev - 1].0, Tok::Op(p) if *p == ")") {
                    let open = skip_back_parens(toks, prev - 1);
                    span_start = open - 2;
                    w = open - 2;
                } else {
                    span_start = prev - 1;
                    w = prev - 1;
                }
            }
            Tok::Kw(p) if MODIFIERS.contains(&&**p) => {
                span_start = prev;
                w = prev;
            }
            Tok::Op(o) if *o == "@" => {
                span_start = prev;
                w = prev;
            }
            _ => break,
        }
    }
    span_start
}

fn body_bounds(toks: &[(Tok<'_>, usize)], term: usize) -> (usize, usize, usize) {
    if matches!(&toks[term].0, Tok::Op(o) if *o == "{") {
        let mut bd = 0usize;
        let mut m = term;
        let mut el = toks[term].1;
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
        (term + 1, m, el)
    } else {
        let prev_line = toks[term.wrapping_sub(1)].1;
        (term, term, prev_line)
    }
}

#[expect(
    clippy::many_single_char_names,
    reason = "the cursor, the current byte and token indices keep their conventional one-letter scanner names (b, c, i, j, k)"
)]
fn constructor_terminator(toks: &[(Tok<'_>, usize)], i: usize) -> Option<usize> {
    if !matches!(toks.get(i + 1), Some((Tok::Op(o), _)) if *o == "(") {
        return None;
    }
    let mut k = i + 2;
    let mut d = 1usize;
    while k < toks.len() && d > 0 {
        match &toks[k].0 {
            Tok::Op(o) if *o == "(" => d += 1,
            Tok::Op(o) if *o == ")" => d -= 1,
            _ => {}
        }
        k += 1;
    }
    if d != 0 {
        return None;
    }
    while k < toks.len() {
        match &toks[k].0 {
            Tok::Op(o) if *o == "{" => return Some(k),
            Tok::Op(o) if *o == "(" => {
                let mut dd = 1usize;
                k += 1;
                while k < toks.len() && dd > 0 {
                    match &toks[k].0 {
                        Tok::Op(p) if *p == "(" => dd += 1,
                        Tok::Op(p) if *p == ")" => dd -= 1,
                        _ => {}
                    }
                    k += 1;
                }
            }
            Tok::Newline => return Some(k),
            _ => k += 1,
        }
    }
    None
}

#[expect(
    clippy::too_many_lines,
    reason = "one pass over the token stream that tracks brace depth, the container stack and the open function span together; a split would share about ten mutable state variables through every helper"
)]
fn analyze_kotlin(src: &str) -> Result<Vec<RawFunction>, String> {
    let (toks, comment_lines) = lex(src)?;
    let mut collector = MetricsCollector::new(&KT_SPEC, 0);
    let mut raws = Vec::new();
    let mut containers: Vec<(Option<String>, usize)> = Vec::new();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < toks.len() {
        match &toks[i].0 {
            Tok::Kw(k) if *k == "fun" => {
                let Some((name, params, j)) = function_head(&toks, i) else {
                    i += 1;
                    continue;
                };
                let owner = containers
                    .iter()
                    .filter_map(|(n, _)| n.clone())
                    .collect::<Vec<_>>()
                    .join("::");
                let qualified = if owner.is_empty() {
                    name
                } else {
                    format!("{owner}::{name}")
                };
                let Some(term) = function_terminator(&toks, j) else {
                    i += 1;
                    continue;
                };
                let span_start = declaration_start(&toks, i);
                let decl_line = toks[span_start].1;
                let io_params = count_u32(params.len() + 1);
                let (body_start, body_end, end_line) = body_bounds(&toks, term);
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
                if matches!(&toks[term].0, Tok::Op(o) if *o == "{") && body_start > i {
                    depth += 1;
                    i = body_start;
                } else {
                    i = i.max(term) + 1;
                }
            }
            Tok::Kw(k) if *k == "constructor" => {
                if containers.last().is_none_or(|(_, d)| *d != depth) {
                    i += 1;
                    continue;
                }
                let Some(term) = constructor_terminator(&toks, i) else {
                    i += 1;
                    continue;
                };
                let owner = containers
                    .iter()
                    .filter_map(|(n, _)| n.clone())
                    .collect::<Vec<_>>()
                    .join("::");
                let qualified = if owner.is_empty() {
                    "constructor".to_string()
                } else {
                    format!("{owner}::constructor")
                };
                let (body_start, body_end, end_line) = body_bounds(&toks, term);
                let raw = build_raw(
                    &mut collector,
                    &toks,
                    &comment_lines,
                    body_start,
                    body_end,
                    toks[i].1,
                    end_line,
                    &qualified,
                    1,
                );
                raws.push(raw);
                if matches!(&toks[term].0, Tok::Op(o) if *o == "{") && body_start > i {
                    depth += 1;
                    i = body_start;
                } else {
                    i = i.max(term) + 1;
                }
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
                    let is_method = matches!(toks.get(idx.wrapping_sub(2)), Some((Tok::Op(p), _)) if *p == "." || *p == "?.");
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
        language: Language::Kotlin,
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
