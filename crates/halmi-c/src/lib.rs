//! halmi-c: C language adapter.
//!
//! Hand lexer + definition scanner on [`halmi_lex`] shared machinery.
//! Zero dependencies, no tree-sitter: the tree-sitter-c oracle is a
//! test-side differential only.
//!
//! * Definitions: `type name(params) { ... }` at file scope, with the
//!   storage class and return type folded into the span (matching
//!   tree-sitter-c's `function_definition`). Prototypes (`;`),
//!   `typedef`s of function pointers, and preprocessor macro bodies
//!   (`preproc_function_def`) are not counted. Old-style K&R parameter
//!   declarations between `)` and `{` are part of the signature.
//! * Operands: identifiers, numbers, strings, chars.
//! * Cyclomatic complexity: 1 + one per `if`, `for`, `while`, `case`,
//!   `&&`, `||` (`do` is not counted; its `while` is).
//! * Physical LOC; nesting = brace depth inside the body (body = 0).
//! * `io_params`: parameter count by last-identifier-before-separator
//!   heuristic + 1 when the return type is not exactly `void`.
//! * `decl_op`: the declarator `(` stands in for C's missing `fn`
//!   keyword (one operator occurrence per function).

#![deny(unsafe_code)]

use halmi_core::adapter::{LanguageAdapter, SourceAnalysis};
use halmi_core::report::{Language, RawFunction, count_f64, count_u32};
use halmi_lex::{LangSpec, LocMode, MetricsCollector};

#[derive(Debug, Clone, Copy, Default)]
pub struct CAdapter;

impl LanguageAdapter for CAdapter {
    fn language(&self) -> &'static str {
        "C"
    }

    fn analyze(&self, path: &str, src: &str) -> SourceAnalysis {
        match analyze_c(src) {
            Ok(mut raws) => {
                for raw in &mut raws {
                    raw.path = path.to_string();
                    raw.language = Language::C;
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

const C_SPEC: LangSpec = LangSpec {
    decl_op: "(",
    keyword_operators: CONTROL_KEYWORDS,
    cc_keywords: &["if", "for", "while", "case"],
    separator_ops: &["(", ")", "[", "]", "{", "}", ",", ";", ":"],
    cc_ops: &["&&", "||"],
    loc_mode: LocMode::Physical,
};

const CONTROL_KEYWORDS: &[&str] = &[
    "if", "else", "for", "while", "do", "switch", "case", "default", "break", "continue", "return",
    "goto", "sizeof",
];

const ALL_KEYWORDS: &[&str] = &[
    "_Bool",
    "_Complex",
    "_Imaginary",
    "auto",
    "break",
    "case",
    "char",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "float",
    "for",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "register",
    "restrict",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "typedef",
    "union",
    "unsigned",
    "void",
    "volatile",
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
];

const TYPEISH: &[&str] = &[
    "const",
    "volatile",
    "static",
    "extern",
    "inline",
    "register",
    "auto",
    "restrict",
    "signed",
    "unsigned",
    "struct",
    "enum",
    "union",
    "typedef",
    "long",
    "short",
    "int",
    "char",
    "float",
    "double",
    "void",
    "_Bool",
    "_Complex",
    "_Imaginary",
];

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

fn lex_string(b: &[u8], q: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = q + 1;
    let mut line = line;
    while i < b.len() {
        match b[i] {
            b'\\' => {
                i += 2;
                if i > 1 && b[i - 1] == b'\n' {
                    line += 1;
                }
            }
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
            b'\n' => return Err(format!("line {line}: newline in character literal")),
            _ => i += 1,
        }
    }
    Err(format!("line {line}: unterminated character literal"))
}

const fn lex_number(b: &[u8], start: usize) -> usize {
    let mut i = start;
    if b[i] == b'0' && i + 1 < b.len() && matches!(b[i + 1], b'x' | b'X') {
        i += 2;
        while i < b.len() && (b[i].is_ascii_hexdigit()) {
            i += 1;
        }
        if i < b.len() && b[i] == b'.' {
            i += 1;
            while i < b.len() && b[i].is_ascii_hexdigit() {
                i += 1;
            }
        }
    } else {
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i < b.len() && b[i] == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
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
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
        }
    }
    while i < b.len() && matches!(b[i], b'u' | b'U' | b'l' | b'L' | b'f' | b'F') {
        i += 1;
    }
    i
}

fn crossable(toks: &[(Tok<'_>, usize)], j: usize) -> bool {
    if typeish(&toks[j].0) {
        return true;
    }
    if matches!(&toks[j].0, Tok::Op(o) if *o == ")") {
        let open = skip_back_parens(toks, j);
        return open > 0
            && matches!(
                &toks[open - 1].0,
                Tok::Ident(n) if matches!(&**n, "__attribute__" | "__declspec" | "__asm__" | "asm")
            );
    }
    false
}

fn skip_back_parens(toks: &[(Tok<'_>, usize)], close: usize) -> usize {
    let mut depth = 1usize;
    let mut j = close;
    while j > 0 {
        j -= 1;
        match &toks[j].0 {
            Tok::Op(o) if *o == ")" => depth += 1,
            Tok::Op(o) if *o == "(" => {
                depth = depth.saturating_sub(1);
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
        Tok::Kw(k) => TYPEISH.contains(&&**k),
        Tok::Op(o) => *o == "*",
        _ => false,
    }
}

fn parameters(
    toks: &[(Tok<'_>, usize)],
    start: usize,
    ln: usize,
    name: &str,
) -> Result<(Vec<String>, usize), String> {
    let mut j = start;
    let mut depth = 1usize;
    let mut params = Vec::new();
    let mut pending: Option<String> = None;
    while j < toks.len() && depth > 0 {
        match &toks[j].0 {
            Tok::Op(o) if *o == "(" => depth += 1,
            Tok::Op(o) if *o == ")" => {
                depth -= 1;
                if depth == 0
                    && let Some(cand) = pending.take()
                    && !params.contains(&cand)
                {
                    params.push(cand);
                }
            }
            Tok::Ident(n) => pending = Some((*n).to_string()),
            Tok::Op(o) if *o == "," && depth == 1 => {
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
    if depth != 0 {
        return Err(format!(
            "line {ln}: unterminated parameter list in `{name}`"
        ));
    }
    let mut k = j;
    while matches!(toks.get(k), Some((Tok::Op(o), _)) if *o == "(") {
        let mut d = 1usize;
        k += 1;
        let mut inner = k;
        while k < toks.len() && d > 0 {
            match &toks[k].0 {
                Tok::Op(p) if *p == "(" => d += 1,
                Tok::Op(p) if *p == ")" => d -= 1,
                _ => {}
            }
            k += 1;
        }
        if d != 0 {
            return Err(format!(
                "line {ln}: unterminated parameter list in `{name}`"
            ));
        }
        params.clear();
        let mut pending: Option<String> = None;
        while inner < k - 1 {
            match &toks[inner].0 {
                Tok::Ident(n) => pending = Some((*n).to_string()),
                Tok::Op(o) if *o == "," => {
                    if let Some(cand) = pending.take()
                        && !params.contains(&cand)
                    {
                        params.push(cand);
                    }
                }
                _ => {}
            }
            inner += 1;
        }
        if let Some(cand) = pending.take()
            && !params.contains(&cand)
        {
            params.push(cand);
        }
    }
    Ok((params, k))
}

fn definition_gap(toks: &[(Tok<'_>, usize)], mut k: usize) -> Option<usize> {
    let mut saw_gap = false;
    let mut since_semi = 0usize;
    while k < toks.len() {
        match &toks[k].0 {
            Tok::Op(o) if *o == "{" => {
                if saw_gap && since_semi > 0 {
                    break;
                }
                return Some(k);
            }
            Tok::Op(o) if *o == ";" => {
                saw_gap = true;
                since_semi = 0;
                k += 1;
            }
            Tok::Op(o) if *o == "(" || *o == "[" => {
                let attr = matches!(
                    toks.get(k.wrapping_sub(1)).map(|t| &t.0),
                    Some(Tok::Ident(n)) if matches!(&**n, "__attribute__" | "__asm__" | "asm" | "__declspec")
                );
                if !attr {
                    break;
                }
                let mut d = 1usize;
                k += 1;
                while k < toks.len() && d > 0 {
                    match &toks[k].0 {
                        Tok::Op(p) if *p == "(" || *p == "[" => d += 1,
                        Tok::Op(p) if *p == ")" || *p == "]" => d -= 1,
                        _ => {}
                    }
                    k += 1;
                }
                saw_gap = true;
            }
            Tok::Op(o) if *o == "," || *o == "*" => {
                saw_gap = true;
                since_semi += 1;
                k += 1;
            }
            Tok::Newline => k += 1,
            Tok::Ident(_) | Tok::Kw(_) => {
                saw_gap = true;
                since_semi += 1;
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
            Tok::Newline if !crossed_newline && w >= 2 && crossable(toks, prev - 1) => {
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

fn c_body_bounds(toks: &[(Tok<'_>, usize)], k: usize) -> (usize, usize) {
    let mut bd = 0usize;
    let mut cond_stack = Vec::new();
    let mut m = k;
    let mut end_line = toks[k].1;
    while m < toks.len() {
        match &toks[m].0 {
            Tok::Op(o) if *o == "{" => bd += 1,
            Tok::Op(o) if *o == "}" => {
                bd = bd.saturating_sub(1);
                if bd == 0 {
                    end_line = toks[m].1;
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
    (m, end_line)
}

fn analyze_c(src: &str) -> Result<Vec<RawFunction>, String> {
    let (toks, comment_lines) = lex(src)?;
    let mut collector = MetricsCollector::new(&C_SPEC, 0);
    let mut raws = Vec::new();
    let mut i = 0usize;
    while i < toks.len() {
        if let (Tok::Ident(name), ln) = &toks[i] {
            if NON_FUNCTIONS.contains(name) {
                i += 1;
                continue;
            }
            if matches!(toks.get(i + 1), Some((Tok::Op(o), _)) if *o == "(") {
                let name = (*name).to_string();
                let (mut params, k) = parameters(&toks, i + 2, *ln, &name)?;
                let Some(k) = definition_gap(&toks, k) else {
                    i += 1;
                    continue;
                };
                if params.len() == 1 && params[0] == "void" {
                    params.clear();
                }
                let span_start = declaration_start(&toks, i);
                let decl_line = toks[span_start].1;
                let mut ret_toks: Vec<&str> = Vec::new();
                for t in &toks[span_start..i] {
                    match &t.0 {
                        Tok::Ident(n) | Tok::Kw(n) => ret_toks.push(n),
                        _ => {}
                    }
                }
                let has_result = !(ret_toks.len() == 1 && ret_toks[0] == "void");
                let io_params = count_u32(params.len() + usize::from(has_result));
                let (m, end_line) = c_body_bounds(&toks, k);
                let raw = build_raw(
                    &mut collector,
                    &toks,
                    &comment_lines,
                    k + 1,
                    m,
                    decl_line,
                    end_line,
                    &name,
                    io_params,
                );
                raws.push(raw);
                i = m + 1;
                continue;
            }
        }
        i += 1;
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
            Tok::Op(o) if o.starts_with('#') => {}
            Tok::Op(o) => match *o {
                "{" => collector.open_block(*ln),
                "}" => collector.close_block(*ln),
                "(" => {
                    let callee = toks.get(idx.wrapping_sub(1)).map(|t| &t.0);
                    let is_method =
                        matches!(toks.get(idx.wrapping_sub(2)), Some((Tok::Op(p), _)) if *p == ".");
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
        language: Language::C,
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
