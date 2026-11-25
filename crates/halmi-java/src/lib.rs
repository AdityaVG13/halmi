//! halmi-java: Java language adapter.
//!
//! Hand lexer + declaration scanner on [`halmi_lex`] shared machinery
//! (same shape as halmi-c/halmi-cpp). Zero dependencies, no
//! tree-sitter: the tree-sitter-java oracle is a test-side
//! differential only.
//!
//! * Definitions: `method_declaration` / `constructor_declaration` /
//!   `static_initializer` parity with tree-sitter-java. Modifier and
//!   annotation runs (`@Override`, `@SuppressWarnings("x")`) fold
//!   into the span. Bodyless methods (`abstract`, `native`,
//!   interface methods) count. Methods of anonymous and local classes
//!   count (the walk descends into bodies). Lambdas and instance
//!   initializer blocks do not.
//! * Candidates are depth-gated to class-body level (`class`,
//!   `interface`, `enum`, `record`, `@interface` containers), which
//!   rejects calls and misfires inside method bodies.
//! * Qualification: `Container::name`; constructors
//!   `Container::Container`; static initializers `Container::<clinit>`.
//! * `decl_op`: the declarator `(` (Java has no `fn` keyword).

#![deny(unsafe_code)]

use halmi_core::adapter::{LanguageAdapter, SourceAnalysis};
use halmi_core::report::{Language, RawFunction, count_f64, count_u32};
use halmi_lex::{LangSpec, LocMode, MetricsCollector};

#[derive(Debug, Clone, Copy, Default)]
pub struct JavaAdapter;

impl LanguageAdapter for JavaAdapter {
    fn language(&self) -> &'static str {
        "Java"
    }

    fn analyze(&self, path: &str, src: &str) -> SourceAnalysis {
        match analyze_java(src) {
            Ok(mut raws) => {
                for raw in &mut raws {
                    raw.path = path.to_string();
                    raw.language = Language::Java;
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

const JAVA_SPEC: LangSpec = LangSpec {
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
    "throw",
    "try",
    "catch",
    "finally",
    "new",
    "instanceof",
    "synchronized",
    "assert",
];

const ALL_KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "package",
    "permits",
    "private",
    "protected",
    "public",
    "record",
    "return",
    "sealed",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "try",
    "var",
    "void",
    "volatile",
    "while",
    "yield",
];

const MODIFIERS: &[&str] = &[
    "public",
    "private",
    "protected",
    "static",
    "final",
    "abstract",
    "synchronized",
    "native",
    "strictfp",
    "default",
    "transient",
    "volatile",
    "sealed",
];

const CONTAINER_KEYWORDS: &[&str] = &["class", "interface", "enum", "record"];

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
                let (end, nl) = lex_text_block(b, i, line)?;
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
            c if c.is_ascii_alphabetic() || c == b'_' || c == b'$' => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$')
                {
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

fn lex_text_block(b: &[u8], q: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = q + 3;
    let mut line = line;
    while i + 2 < b.len() {
        if b[i] == b'\\' {
            if i + 1 < b.len() && b[i + 1] == b'\n' {
                line += 1;
            } else if i + 2 < b.len() && b[i + 1] == b'\r' && b[i + 2] == b'\n' {
                line += 1;
                i += 1;
            }
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
    Err(format!("line {line}: unterminated text block"))
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
            Tok::Op(o) if *o == ">>>" => {
                if depth <= 3 {
                    return k + 1;
                }
                depth -= 3;
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
/// `[modifiers] class|interface|enum|record|@interface [NAME] [<...>]`.
#[expect(
    clippy::too_many_lines,
    reason = "walks backwards over `[modifiers] class|interface|enum|record [NAME] [<...>]` in one place; the arms mirror that grammar"
)]
fn container_name_for(toks: &[(Tok<'_>, usize)], brace: usize) -> Container {
    let mut j = brace;
    if j == 0 {
        return Container::Not;
    }
    j -= 1;
    if matches!(&toks[j].0, Tok::Op(o) if *o == ">" || *o == ">>" || *o == ">>>") {
        let open = skip_back_angles(toks, j);
        if open == 0 || open > j {
            return Container::Not;
        }
        j = open - 1;
    }
    let mut name: Option<String> = None;
    if let Tok::Ident(n) = &toks[j].0 {
        name = Some((*n).to_string());
        if j == 0 {
            return Container::Body(match &toks[0].0 {
                Tok::Kw(k) if CONTAINER_KEYWORDS.contains(&&**k) => name,
                _ => None,
            });
        }
        j -= 1;
        if matches!(&toks[j].0, Tok::Op(o) if *o == ",") {
            return Container::Body(name);
        }
        while j > 0 {
            match &toks[j].0 {
                Tok::Newline | Tok::Ident(_) => j -= 1,
                Tok::Kw(k) if matches!(&**k, "extends" | "implements") => j -= 1,
                Tok::Op(o) if *o == "," || *o == "." || *o == "&" => j -= 1,
                Tok::Op(o) if *o == ">" || *o == ">>" || *o == ">>>" => {
                    let open = skip_back_angles(toks, j);
                    if open == 0 || open > j {
                        break;
                    }
                    j = open - 1;
                }
                _ => break,
            }
        }
    }
    match &toks[j].0 {
        Tok::Op(o) if *o == ")" => {
            let anon = (|| {
                let open = skip_back_parens(toks, j);
                if open == 0 {
                    return false;
                }
                let mut k = open;
                if k == 0 {
                    return false;
                }
                k -= 1;
                if matches!(&toks[k].0, Tok::Op(p) if *p == ">" || *p == ">>" || *p == ">>>") {
                    let open2 = skip_back_angles(toks, k);
                    if open2 == 0 || open2 > k {
                        return false;
                    }
                    k = open2 - 1;
                }
                if !matches!(&toks[k].0, Tok::Ident(_)) {
                    return false;
                }
                loop {
                    if k == 0 {
                        return false;
                    }
                    k -= 1;
                    match &toks[k].0 {
                        Tok::Op(p) if *p == "." => {
                            if k == 0 {
                                return false;
                            }
                            k -= 1;
                            if !matches!(&toks[k].0, Tok::Ident(_)) {
                                return false;
                            }
                        }
                        Tok::Kw(p) if *p == "new" => return true,
                        _ => return false,
                    }
                }
            })();
            if anon {
                return Container::Body(None);
            }
            let open = skip_back_parens(toks, j);
            let record = open > 1
                && matches!(&toks[open - 1].0, Tok::Ident(_))
                && matches!(&toks[open.wrapping_sub(2)].0, Tok::Kw(p) if *p == "record");
            if record {
                Container::Body(match &toks[open - 1].0 {
                    Tok::Ident(n) => Some((*n).to_string()),
                    _ => None,
                })
            } else {
                Container::Not
            }
        }
        Tok::Kw(k) if CONTAINER_KEYWORDS.contains(&&**k) => Container::Body(name),
        Tok::Op(o) if *o == "@" => {
            if matches!(&toks.get(j.wrapping_sub(1)).map(|t| &t.0), Some(Tok::Kw(p)) if *p == "interface")
            {
                Container::Body(name)
            } else {
                Container::Not
            }
        }
        _ => Container::Not,
    }
}

/// The name of an enum constant whose body brace is at `brace`
/// (`CONST`, `CONST {`, or `CONST(args) {`).
fn enum_constant_name(toks: &[(Tok<'_>, usize)], brace: usize) -> Option<String> {
    let mut j = brace;
    if j == 0 {
        return None;
    }
    j -= 1;
    if matches!(&toks[j].0, Tok::Op(o) if *o == ")") {
        let open = skip_back_parens(toks, j);
        if open == 0 || open > j {
            return None;
        }
        j = open - 1;
    }
    match &toks[j].0 {
        Tok::Ident(n) => Some((*n).to_string()),
        _ => None,
    }
}

/// Whether the container whose brace is at `brace` is an `enum`
/// (constants section until the member-level `;`).
fn container_is_enum(toks: &[(Tok<'_>, usize)], brace: usize) -> bool {
    if matches!(container_name_for(toks, brace), Container::Not) {
        return false;
    }
    let mut j = brace;
    if j == 0 {
        return false;
    }
    j -= 1;
    if matches!(&toks[j].0, Tok::Op(o) if *o == ">") {
        let open = skip_back_angles(toks, j);
        if open == 0 || open > j {
            return false;
        }
        j = open - 1;
    }
    if let Tok::Op(o) = &toks[j].0
        && *o == ")"
    {
        let open = skip_back_parens(toks, j);
        if open == 0 || open > j {
            return false;
        }
        j = open - 1;
    }
    if let Tok::Ident(_) = &toks[j].0 {
        if j == 0 {
            return false;
        }
        j -= 1;
    }
    while j > 0 {
        match &toks[j].0 {
            Tok::Newline | Tok::Ident(_) => j -= 1,
            Tok::Kw(k) if matches!(&**k, "extends" | "implements") => j -= 1,
            Tok::Op(o) if *o == "," || *o == "." || *o == "&" => j -= 1,
            Tok::Op(o) if *o == ">" || *o == ">>" || *o == ">>>" => {
                let open = skip_back_angles(toks, j);
                if open == 0 || open > j {
                    break;
                }
                j = open - 1;
            }
            _ => break,
        }
    }
    matches!(&toks[j].0, Tok::Kw(k) if *k == "enum")
}

fn annotation_close(toks: &[(Tok<'_>, usize)], j: usize) -> bool {
    if !matches!(&toks[j].0, Tok::Op(o) if *o == ")") {
        return false;
    }
    let open = skip_back_parens(toks, j);
    open > 1
        && matches!(&toks[open - 1].0, Tok::Ident(_))
        && matches!(&toks[open.wrapping_sub(2)].0, Tok::Op(p) if *p == "@")
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
                        | "byte"
                        | "char"
                        | "float"
                        | "double"
                        | "boolean"
                        | "void"
                        | "var"
                        | "extends"
                        | "implements"
                        | "throws"
                        | "super"
                )
        }
        Tok::Op(o) => matches!(&**o, "*" | "&" | "::" | "..." | "[" | "]"),
        _ => false,
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

#[expect(
    clippy::many_single_char_names,
    reason = "the cursor, the current byte and token indices keep their conventional one-letter scanner names (b, c, i, j, k)"
)]
fn method_terminator(toks: &[(Tok<'_>, usize)], i: usize, mut k: usize) -> Option<(usize, bool)> {
    while k < toks.len() {
        match &toks[k].0 {
            Tok::Op(o) if *o == "{" => return Some((k, true)),
            Tok::Op(o) if *o == ";" => {
                let type_before = toks.get(i.wrapping_sub(1)).is_some_and(|t| typeish(&t.0));
                if type_before {
                    return Some((k, false));
                }
                break;
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
            Tok::Op(o) if *o == "->" || *o == "," || *o == "*" || *o == "::" || *o == "." => {
                k += 1;
            }
            Tok::Newline | Tok::Ident(_) => k += 1,
            Tok::Kw(kw) => {
                if typeish(&Tok::Kw(kw)) {
                    k += 1;
                } else {
                    break;
                }
            }
            Tok::Op(o) if *o == ">" || *o == ">>" => k += 1,
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
            Tok::Op(o) if *o == ">" || *o == ">>" || *o == ">>>" => {
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
                    && matches!(&toks[open - 1].0, Tok::Ident(_))
                    && matches!(&toks[open.wrapping_sub(2)].0, Tok::Op(p) if *p == "@");
                if attr {
                    span_start = open - 2;
                    w = open - 2;
                    crossed_newline = false;
                } else {
                    break;
                }
            }
            Tok::Op(o) if *o == "@" => {
                span_start = prev;
                w = prev;
                crossed_newline = false;
            }
            Tok::Newline
                if !crossed_newline
                    && w >= 2
                    && (typeish(&toks[prev - 1].0)
                        || matches!(&toks[prev - 1].0, Tok::Op(p) if *p == "@")
                        || matches!(&toks[prev - 1].0, Tok::Op(p) if *p == ">" || *p == ">>")
                        || annotation_close(toks, prev - 1)) =>
            {
                crossed_newline = true;
                if annotation_close(toks, prev - 1) {
                    let open = skip_back_parens(toks, prev - 1);
                    span_start = open - 2;
                    w = open - 2;
                    crossed_newline = false;
                } else {
                    span_start = prev - 1;
                    w = prev - 1;
                }
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

fn java_body_bounds(toks: &[(Tok<'_>, usize)], k: usize, brace: bool) -> (usize, usize, usize) {
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
fn analyze_java(src: &str) -> Result<Vec<RawFunction>, String> {
    let (toks, comment_lines) = lex(src)?;
    let mut collector = MetricsCollector::new(&JAVA_SPEC, 0);
    let mut raws = Vec::new();
    let mut containers: Vec<(Option<String>, usize, bool)> = Vec::new();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < toks.len() {
        match &toks[i].0 {
            Tok::Ident(name)
                if matches!(toks.get(i + 1), Some((Tok::Op(o), _)) if *o == "(" || *o == "<")
                    && containers.last().is_some_and(|(_, d, _)| *d == depth)
                    && containers.last().is_some_and(|(_, _, done)| *done)
                    && !matches!(toks.get(i.wrapping_sub(1)), Some((Tok::Op(o), _)) if *o == "@" || *o == "." || *o == "::")
                    && !matches!(toks.get(i.wrapping_sub(1)), Some((Tok::Kw(o), _)) if *o == "new") =>
            {
                let mut j = i + 1;
                if matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "<") {
                    let end = skip_angles_fwd(&toks, j);
                    if end == 0 || !matches!(toks.get(end), Some((Tok::Op(o), _)) if *o == "(") {
                        i += 1;
                        continue;
                    }
                    j = end;
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
                let Some((params, j)) = method_parameters(&toks, j) else {
                    i += 1;
                    continue;
                };
                let Some((k, terminator_brace)) = method_terminator(&toks, i, j) else {
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
                let (body_start, body_end, end_line) = java_body_bounds(&toks, k, terminator_brace);
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
                if *k == "static"
                    && matches!(toks.get(i + 1), Some((Tok::Op(o), _)) if *o == "{")
                    && containers.last().is_some_and(|(_, d, _)| *d == depth) =>
            {
                let owner = containers
                    .last()
                    .and_then(|(n, _, _)| n.clone())
                    .unwrap_or_else(|| "<clinit>".to_string());
                let qualified = format!("{owner}::<clinit>");
                let (body_start, body_end, end_line) = java_body_bounds(&toks, i + 1, true);
                let raw = build_raw(
                    &mut collector,
                    &toks,
                    &comment_lines,
                    body_start,
                    body_end,
                    toks[i].1,
                    end_line,
                    &qualified,
                    0,
                );
                raws.push(raw);
                i = body_end + 1;
            }
            Tok::Op(o) if *o == "{" => {
                let mut container = container_name_for(&toks, i);
                let enum_name = (matches!(container, Container::Not)
                    && containers
                        .last()
                        .is_some_and(|(_, d, done)| *d == depth && !*done))
                .then(|| enum_constant_name(&toks, i))
                .flatten();
                let enum_fallback = enum_name.is_some();
                container = enum_name.map_or(container, |name| Container::Body(Some(name)));
                depth += 1;
                if let Container::Body(c) = container {
                    let is_enum = !enum_fallback && container_is_enum(&toks, i);
                    containers.push((c, depth, !is_enum));
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
        language: Language::Java,
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
