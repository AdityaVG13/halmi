//! halmi-python: the Python adapter for halmi.
//!
//! A hand-written lexer (no parser dependency) feeds a single structure
//! pass that yields one [`RawFunction`] per `def` (methods qualified
//! `Class.method`). The contract is the shared [`LanguageAdapter`] port:
//! pure, never panics, one pass, degraded files become error entries.
//!
//! Classification (token-faithful; see CONTRACT.md):
//! * Operators: symbol operators (`+ - * ** / // % @ = += ... << >> & | ^
//!   ~ < > <= >= == !=`), attribute access `.`, invocation `call`,
//!   subscript `index`, and the keywords `def if elif for while return
//!   break continue lambda yield raise with and or not in is match`.
//! * Operands: identifiers that are not keywords, numbers, and string
//!   literals (an f-string is one operand; interpolations are not lexed).
//! * Cyclomatic complexity: 1 + one per `if`, `elif`, `for`, `while`,
//!   `except`, `and`, `or`, `case` (ternary `if` counts via `if`).
//! * Logical LOC: logical lines (continuations and open brackets join
//!   lines), comments and blanks excluded.
//! * Nesting: indentation depth relative to the function body (body = 1).
//! * Nested `def`/`class` inside a body: excluded from the parent's
//!   counts and not reported, matching the Rust adapter.
//! * Spans: the `def` line through the last body line; decorators and
//!   blank trailing lines are excluded.
//! * `io_params`: distinct parameter names plus one for a `->` return
//!   annotation. `method_chain`: consecutive called attribute accesses
//!   (`x.m1().m2()`), receiver spine only.

#![deny(unsafe_code)]

use halmi_core::adapter::{LanguageAdapter, SourceAnalysis};
use halmi_core::report::{RawFunction, count_u32};
use halmi_lex::MetricsCollector;

/// The Python adapter.
#[derive(Debug, Clone, Copy, Default)]
pub struct PythonAdapter;

const KEYWORD_OPERATORS: [&str; 17] = [
    "def", "if", "elif", "for", "while", "return", "break", "continue", "lambda", "yield", "raise",
    "with", "and", "or", "not", "in", "is",
];
const ALL_KEYWORDS: [&str; 32] = [
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda",
    "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
];

impl LanguageAdapter for PythonAdapter {
    fn language(&self) -> &'static str {
        "Python"
    }

    fn analyze(&self, path: &str, src: &str) -> SourceAnalysis {
        match analyze_python(src) {
            Ok(mut raws) => {
                for raw in &mut raws {
                    raw.path = path.to_string();
                    raw.language = halmi_core::report::Language::Python;
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

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok<'a> {
    Ident(&'a str),
    Number(&'a str),
    Str(&'a str),
    Op(&'a str),
    Kw(&'a str),
    Newline,
    Indent,
    Dedent,
}

#[derive(Debug)]
struct Lexed<'a> {
    toks: Vec<(Tok<'a>, usize)>,
    /// true when physical line `i` (1-based) holds comment text.
    comment_lines: Vec<bool>,
}

const fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

fn indent_width(line: &str) -> usize {
    let mut w = 0;
    for c in line.chars() {
        match c {
            ' ' => w += 1,
            '\t' => w += 8 - (w % 8),
            _ => break,
        }
    }
    w
}

const fn prefix_quote(b: &[u8], i: usize) -> Option<(bool, usize)> {
    let mut j = i;
    let mut raw = false;
    while j < b.len() && matches!(b[j], b'r' | b'b' | b'f' | b'u' | b'R' | b'B' | b'F' | b'U') {
        if b[j] == b'r' || b[j] == b'R' {
            raw = true;
        }
        j += 1;
    }
    if j > i && j < b.len() && (b[j] == b'\'' || b[j] == b'"') {
        Some((raw, j))
    } else {
        None
    }
}

fn scan_quoted(src: &str, q: usize, raw: bool, line: usize) -> Result<(usize, usize), String> {
    let b = src.as_bytes();
    let quote = b[q];
    let mut line = line;
    let mut k = q + 1;
    let triple = k + 1 < b.len() && b[k] == quote && b[k + 1] == quote;
    if triple {
        k += 2;
    }
    loop {
        if k >= b.len() {
            return Err(format!("line {line}: unterminated string"));
        }
        if b[k] == b'\\' {
            if !raw {
                k += 2;
                continue;
            }
            let mut run = 0;
            while k + run < b.len() && b[k + run] == b'\\' {
                run += 1;
            }
            if k + run < b.len() && b[k + run] == quote && run % 2 == 1 {
                k += run + 1;
                continue;
            }
            k += run;
            continue;
        }
        if b[k] == b'\n' {
            if triple {
                line += 1;
                k += 1;
                continue;
            }
            return Err(format!("line {line}: unterminated string"));
        }
        if b[k] == quote {
            if triple {
                if k + 2 < b.len() && b[k + 1] == quote && b[k + 2] == quote {
                    k += 3;
                    break;
                }
                k += 1;
                continue;
            }
            k += 1;
            break;
        }
        k += 1;
    }
    Ok((k, line))
}

const fn lex_number(src: &str, start: usize) -> usize {
    let b = src.as_bytes();
    let mut i = start;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_alphanumeric() || c == b'_' {
            i += 1;
            continue;
        }
        if c == b'.' {
            if i + 1 < b.len() && b[i + 1] == b'.' {
                break;
            }
            i += 1;
            continue;
        }
        if (c == b'+' || c == b'-') && i > start && matches!(b[i - 1], b'e' | b'E') {
            i += 1;
            continue;
        }
        break;
    }
    i
}

fn lex_word(src: &str, start: usize) -> (Tok<'_>, usize) {
    let b = src.as_bytes();
    let mut i = start;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
        i += 1;
    }
    while i < b.len() && b[i] >= 0x80 {
        i += utf8_len(b[i]);
        while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
            i += 1;
        }
    }
    let word = &src[start..i];
    let tok = if ALL_KEYWORDS.binary_search(&word).is_ok() {
        Tok::Kw(word)
    } else {
        Tok::Ident(word)
    };
    (tok, i)
}

fn punct_length(src: &str, i: usize, line: usize) -> Result<usize, String> {
    let b = src.as_bytes();
    let c = b[i];
    let two = if i + 2 <= b.len() && src.is_char_boundary(i + 2) {
        &src[i..i + 2]
    } else {
        ""
    };
    let three = if i + 3 <= b.len() && src.is_char_boundary(i + 3) {
        &src[i..i + 3]
    } else {
        ""
    };
    if matches!(three, "**=" | "//=" | "<<=" | ">>=" | "...") {
        return Ok(3);
    }
    if matches!(
        two,
        "**" | "//"
            | "=="
            | "!="
            | "<="
            | ">="
            | "+="
            | "-="
            | "*="
            | "/="
            | "%="
            | "&="
            | "|="
            | "^="
            | "<<"
            | ">>"
            | "->"
            | ":="
            | "..."
    ) {
        return Ok(2);
    }
    if matches!(
        c,
        b'+' | b'-'
            | b'*'
            | b'/'
            | b'%'
            | b'@'
            | b'='
            | b'<'
            | b'>'
            | b'&'
            | b'|'
            | b'^'
            | b'~'
            | b'('
            | b')'
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b','
            | b':'
            | b';'
            | b'.'
            | b'$'
            | b'?'
            | b'!'
    ) {
        return Ok(1);
    }
    Err(format!("line {line}: unexpected character `{}`", c as char))
}

#[expect(
    clippy::too_many_lines,
    reason = "single-pass byte scanner: one match over the token grammar keeps every lookahead rule beside its neighbours; a split would thread the cursor, line counter and token buffer through every helper"
)]
#[expect(
    clippy::many_single_char_names,
    reason = "the cursor, the current byte and token indices keep their conventional one-letter scanner names (b, c, i, j, k)"
)]
fn lex(src: &str) -> Result<Lexed<'_>, String> {
    let b = src.as_bytes();
    let n_lines = src.lines().count().max(1);
    let mut comment_lines = vec![false; n_lines];
    let mut toks: Vec<(Tok, usize)> = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut indents: Vec<usize> = vec![0];
    let mut at_line_start = true;
    let mut bracket_depth = 0usize;

    while i < b.len() {
        let c = b[i];
        if at_line_start && bracket_depth == 0 {
            let rest = &src[i..];
            let width = indent_width(rest.split('\n').next().unwrap_or(""));
            let content = rest.split('\n').next().unwrap_or("").trim_start();
            if content.is_empty() || content.starts_with('#') {
                if content.starts_with('#') && line <= n_lines {
                    comment_lines[line - 1] = true;
                }
                let nl = rest.find('\n').map_or(rest.len(), |p| p + 1);
                line += 1;
                i += nl;
                continue;
            }
            at_line_start = false;
            let cur = *indents.last().unwrap();
            if width > cur {
                indents.push(width);
                toks.push((Tok::Indent, line));
            } else if width < cur {
                while *indents.last().unwrap() > width {
                    indents.pop();
                    toks.push((Tok::Dedent, line));
                }
                if *indents.last().unwrap() != width {
                    return Err(format!("line {line}: inconsistent dedent"));
                }
            }
            continue;
        }
        match c {
            b'\n' => {
                if bracket_depth > 0 {
                    line += 1;
                    i += 1;
                    continue;
                }
                toks.push((Tok::Newline, line));
                line += 1;
                i += 1;
                at_line_start = true;
            }
            b' ' | b'\t' | b'\r' => i += 1,
            b'#' => {
                if line <= n_lines {
                    comment_lines[line - 1] = true;
                }
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'\\' if i + 1 < b.len() && b[i + 1] == b'\n' => {
                line += 1;
                i += 2;
            }
            b'\'' | b'"' => {
                let (k, new_line) = scan_quoted(src, i, false, line)?;
                line = new_line;
                toks.push((Tok::Str(&src[i..k]), line));
                i = k;
            }
            b'r' | b'b' | b'f' | b'u' | b'R' | b'B' | b'F' | b'U'
                if prefix_quote(b, i).is_some() =>
            {
                let (raw, q) = prefix_quote(b, i).unwrap();
                let (k, new_line) = scan_quoted(src, q, raw, line)?;
                line = new_line;
                toks.push((Tok::Str(&src[i..k]), line));
                i = k;
            }
            b'0'..=b'9' => {
                let start = i;
                let end = lex_number(src, start);
                toks.push((Tok::Number(&src[start..end]), line));
                i = end;
            }
            b'_' | b'a'..=b'z' | b'A'..=b'Z' => {
                let (tok, end) = lex_word(src, i);
                toks.push((tok, line));
                i = end;
            }
            _ => {
                if !c.is_ascii() {
                    let len = utf8_len(c);
                    toks.push((Tok::Op(&src[i..i + len]), line));
                    i += len;
                    continue;
                }
                let op = punct_length(src, i, line)?;
                match c {
                    b'(' | b'[' | b'{' => bracket_depth += 1,
                    b')' | b']' | b'}' => bracket_depth = bracket_depth.saturating_sub(1),
                    _ => {}
                }
                toks.push((Tok::Op(&src[i..i + op]), line));
                i += op;
            }
        }
    }
    Ok(Lexed {
        toks,
        comment_lines,
    })
}

#[expect(
    clippy::too_many_lines,
    reason = "one pass over the token stream that tracks brace depth, the container stack and the open function span together; a split would share about ten mutable state variables through every helper"
)]
fn analyze_python(src: &str) -> Result<Vec<RawFunction>, String> {
    let lexed = lex(src)?;
    let mut collector = MetricsCollector::new(&PY_SPEC, 0);
    let toks = &lexed.toks;
    let mut raws = Vec::new();

    let mut scopes: Vec<(String, usize)> = Vec::new();
    let mut i = 0usize;
    while i < toks.len() {
        while scopes.last().is_some_and(|(_, end)| *end <= i) {
            scopes.pop();
        }
        let (tok, ln) = &toks[i];
        match tok {
            Tok::Kw(k) if *k == "def" || *k == "class" => {
                let name = match toks.get(i + 1) {
                    Some((Tok::Ident(n), _)) => (*n).to_string(),
                    _ => return Err(format!("line {ln}: {k} without a name")),
                };
                let mut j = i + 1;
                let mut header_parens = 0usize;

                let mut seen_arrow = false;
                let mut param_names: Vec<String> = Vec::new();
                let mut expect_param = false;
                while j < toks.len() {
                    match &toks[j].0 {
                        Tok::Op(o) if *o == "(" => {
                            header_parens += 1;
                            expect_param = header_parens == 1;
                        }
                        Tok::Op(o) if *o == ")" => header_parens -= 1,
                        Tok::Op(o) if *o == "," && header_parens == 1 => expect_param = true,
                        Tok::Op(o) if *o == ":" && header_parens == 0 => break,
                        Tok::Op(o) if *o == "->" => seen_arrow = true,
                        Tok::Ident(n) if expect_param => {
                            if !param_names.iter().any(|p| p == *n) {
                                param_names.push((*n).to_string());
                            }
                            expect_param = false;
                        }
                        _ => {}
                    }
                    j += 1;
                }
                let io_params: usize = param_names.len() + usize::from(seen_arrow);

                if *k == "class" {
                    let mut b = j + 1;
                    while b < toks.len() && matches!(toks[b].0, Tok::Newline) {
                        b += 1;
                    }
                    let end = if matches!(toks.get(b).map(|t| &t.0), Some(Tok::Indent)) {
                        let mut depth = 0usize;
                        let mut k2 = b;
                        while k2 < toks.len() {
                            match toks[k2].0 {
                                Tok::Indent => depth += 1,
                                Tok::Dedent => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            k2 += 1;
                        }
                        k2
                    } else {
                        b
                    };
                    scopes.push((name, end));
                    i = j;
                    continue;
                }

                let one_line_suite = !matches!(
                    toks.get(j + 1).map(|t| &t.0),
                    Some(Tok::Indent | Tok::Newline)
                );
                let mut b = j;
                if !one_line_suite {
                    while b < toks.len() && !matches!(toks[b].0, Tok::Indent | Tok::Newline) {
                        b += 1;
                    }
                }
                let body_start = if one_line_suite { j + 1 } else { b + 1 };
                let mut body_end = body_start;
                let mut depth = 0usize;
                let mut end_line = *ln;
                let mut k2 = body_start;
                let mut nested_skip: usize = 0;
                while k2 < toks.len() {
                    match &toks[k2].0 {
                        Tok::Kw(kk)
                            if (*kk == "def" || *kk == "class")
                                && depth == 0
                                && nested_skip == 0 =>
                        {
                            break;
                        }
                        Tok::Indent => depth += 1,
                        Tok::Dedent => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {
                            if nested_skip == 0 {
                                end_line = toks[k2].1;
                                body_end = k2;
                            }
                        }
                    }
                    if let (Tok::Kw(kk), _) = &toks[k2]
                        && (*kk == "def" || *kk == "class")
                        && nested_skip == 0
                    {
                        nested_skip = 1;
                    }
                    if nested_skip > 0 {
                        match &toks[k2].0 {
                            Tok::Indent => nested_skip += 1,
                            Tok::Dedent => {
                                nested_skip -= 1;
                            }
                            _ => {}
                        }
                        if nested_skip == 0 {
                            k2 += 1;
                            continue;
                        }
                    }
                    k2 += 1;
                }
                let qualified = qualified_name(&scopes, &name);
                raws.push(build_raw(
                    &mut collector,
                    &lexed,
                    &scopes,
                    body_start,
                    body_end,
                    *ln,
                    end_line,
                    &qualified,
                    count_u32(io_params),
                ));
                i = k2;
            }
            _ => i += 1,
        }
    }
    Ok(raws)
}

fn qualified_name(scopes: &[(String, usize)], name: &str) -> String {
    if scopes.is_empty() {
        name.to_string()
    } else {
        let owners: Vec<&str> = scopes.iter().map(|(n, _)| n.as_str()).collect();
        format!("{}::{}", owners.join("::"), name)
    }
}

#[allow(clippy::too_many_arguments)]
fn build_raw<'a>(
    collector: &mut MetricsCollector<'a>,

    lexed: &'a Lexed<'a>,
    _scopes: &[(String, usize)],
    body_start: usize,
    body_end: usize,
    def_line: usize,
    end_line: usize,
    name: &str,
    io_params: u32,
) -> RawFunction {
    let toks = &lexed.toks[..=body_end.min(lexed.toks.len().saturating_sub(1))];
    for (idx, (tok, ln)) in toks.iter().enumerate().skip(body_start) {
        match tok {
            Tok::Newline => collector.token(&halmi_lex::Tok::Newline, *ln),
            Tok::Indent => {
                let depth = collector.current_depth() + 1;
                collector.set_depth(depth);
            }
            Tok::Dedent => {
                let depth = collector.current_depth().saturating_sub(1);
                collector.set_depth(depth);
            }
            Tok::Ident(n) => collector.token(&halmi_lex::Tok::Ident(n), *ln),
            Tok::Number(n) | Tok::Str(n) => collector.token(&halmi_lex::Tok::Str(n), *ln),
            Tok::Kw(k) => collector.token(&halmi_lex::Tok::Keyword(k), *ln),
            Tok::Op(o) => match *o {
                "." => collector.token(&halmi_lex::Tok::Punct("."), *ln),
                "(" => {
                    let callee = toks.get(idx.wrapping_sub(1)).map(|t| &t.0);
                    let is_call = match callee {
                        Some(Tok::Ident(_)) => true,
                        Some(Tok::Op(p)) => *p == ")" || *p == "]",
                        _ => false,
                    };
                    if is_call {
                        let is_method = matches!(
                            toks.get(idx.wrapping_sub(2)).map(|t| &t.0),
                            Some(Tok::Op(p)) if *p == "."
                        );
                        let extends = matches!(
                            toks.get(idx.wrapping_sub(3)).map(|t| &t.0),
                            Some(Tok::Op(p)) if *p == ")" || *p == "]"
                        );
                        collector.call(is_method, extends);
                    } else {
                        collector.token(&halmi_lex::Tok::Punct("("), *ln);
                    }
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
    let mut outcome = collector.finish(io_params as usize);
    if outcome.nesting == 0 {
        outcome.nesting = 1;
    }
    RawFunction {
        path: String::new(),
        language: halmi_core::report::Language::Python,
        name: name.to_string(),
        start_line: count_u32(def_line),
        end_line: count_u32(end_line),
        counts: outcome.counts,
        cyclomatic: outcome.cyclomatic,
        loc: outcome.loc,
        nesting: outcome.nesting,
        comment_density: Some(halmi_lex::density(
            &lexed.comment_lines,
            count_u32(def_line),
            count_u32(end_line),
        )),
        method_chain: outcome.method_chain,
    }
}

const PY_SPEC: halmi_lex::LangSpec = halmi_lex::LangSpec {
    decl_op: "def",
    keyword_operators: &KEYWORD_OPERATORS,
    cc_keywords: &["if", "elif", "for", "while", "except", "and", "or"],
    separator_ops: &["(", ")", "[", "]", "{", "}", ",", ";", ":"],
    cc_ops: &[],
    loc_mode: halmi_lex::LocMode::Logical,
};

#[cfg(test)]
mod kw_tests {
    #[test]
    fn keywords_sorted_for_binary_search() {
        assert!(super::ALL_KEYWORDS.windows(2).all(|w| w[0] <= w[1]));
    }
}
