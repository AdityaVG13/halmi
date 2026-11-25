//! halmi-go: the Go adapter for halmi.
//!
//! Hand-written lexer, no parser dependency. One [`RawFunction`] per
//! top-level `func` declaration; methods are qualified `Type.method`.
//! Contract: the shared [`LanguageAdapter`] port (pure, never panics,
//! degraded files become error entries).
//!
//! Classification (token-faithful; see CONTRACT.md):
//! * Operators: Go symbol operators (`+ - * / % & | ^ << >> &^ && || !
//!   == != < <= > >= = := += ... <- ++ -- ...`), `.`, `call`, `index`, and
//!   the keywords `func if else for range switch case select return break
//!   continue go defer chan map struct interface`.
//! * Operands: identifiers that are not keywords, numbers, strings
//!   (interpreted and backtick raw), and runes.
//! * Cyclomatic complexity: 1 + one per `if`, `for`, `case`, `select`,
//!   `&&`, `||`.
//! * Logical LOC: non-blank, non-comment lines in the span.
//! * Nesting: brace depth inside the body (body = 0).
//! * Function literals (closures) count inside their parent, like the Rust
//!   adapter. `io_params`: distinct parameter names plus 1 when the result
//!   list is non-empty. `method_chain`: consecutive called selectors.

#![deny(unsafe_code)]

use halmi_core::adapter::{LanguageAdapter, SourceAnalysis};
use halmi_core::report::{RawFunction, count_f64, count_u32};
use halmi_lex::MetricsCollector;

/// The Go adapter.
#[derive(Debug, Clone, Copy, Default)]
pub struct GoAdapter;

const CONTROL_KEYWORDS: [&str; 13] = [
    "func", "if", "else", "for", "range", "switch", "case", "select", "return", "break",
    "continue", "go", "defer",
];
const ALL_KEYWORDS: [&str; 24] = [
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
];

impl LanguageAdapter for GoAdapter {
    fn language(&self) -> &'static str {
        "Go"
    }

    fn analyze(&self, path: &str, src: &str) -> SourceAnalysis {
        match analyze_go(src) {
            Ok(mut raws) => {
                for raw in &mut raws {
                    raw.path = path.to_string();
                    raw.language = halmi_core::report::Language::Go;
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
    let mut i = 0usize;
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
            b'"' => {
                let (ni, nl) = lex_interpreted_string(b, i, line)?;
                toks.push((Tok::Str(&src[i..ni]), line));
                line = nl;
                i = ni;
            }
            b'`' => {
                let mut j = i + 1;
                while j < b.len() && b[j] != b'`' {
                    if b[j] == b'\n' {
                        line += 1;
                    }
                    j += 1;
                }
                if j >= b.len() {
                    return Err(format!("line {line}: unterminated raw string"));
                }
                toks.push((Tok::Str(&src[i..=j]), line));
                i = j + 1;
            }
            b'\'' => {
                let mut j = i + 1;
                while j < b.len() && b[j] != b'\'' {
                    if b[j] == b'\\' {
                        j += 1;
                    }
                    j += 1;
                }
                if j >= b.len() {
                    return Err(format!("line {line}: unterminated rune"));
                }
                toks.push((Tok::Rune(&src[i..=j]), line));
                i = j + 1;
            }
            b'0'..=b'9' => {
                let start = i;
                while i < b.len()
                    && (b[i].is_ascii_alphanumeric()
                        || b[i] == b'.'
                        || b[i] == b'_'
                        || b[i] == b'+'
                        || b[i] == b'-')
                {
                    if (b[i] == b'+' || b[i] == b'-')
                        && !matches!(b.get(i.wrapping_sub(1)), Some(b'e' | b'E' | b'p' | b'P'))
                    {
                        break;
                    }
                    if b[i] == b'.' && b.get(i + 1) == Some(&b'.') {
                        break;
                    }
                    i += 1;
                }
                toks.push((Tok::Number(&src[start..i]), line));
            }
            b'_' | b'a'..=b'z' | b'A'..=b'Z' => {
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
                if !c.is_ascii() {
                    let len = utf8_len(c);
                    toks.push((Tok::Op(&src[i..i + len]), line));
                    i += len;
                    continue;
                }
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
                let op_len = if matches!(three, "&^=" | "...") {
                    3
                } else if matches!(
                    two,
                    "==" | "!="
                        | "<="
                        | ">="
                        | "&&"
                        | "||"
                        | "<<="
                        | ">>="
                        | "&^"
                        | ":="
                        | "<-"
                        | "++"
                        | "--"
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
                ) {
                    2
                } else if matches!(
                    c,
                    b'+' | b'-'
                        | b'*'
                        | b'/'
                        | b'%'
                        | b'&'
                        | b'|'
                        | b'^'
                        | b'<'
                        | b'>'
                        | b'!'
                        | b'='
                        | b'('
                        | b')'
                        | b'['
                        | b']'
                        | b'{'
                        | b'}'
                        | b','
                        | b';'
                        | b'.'
                        | b':'
                        | b'~'
                ) {
                    1
                } else {
                    return Err(format!("line {line}: unexpected character `{}`", c as char));
                };
                toks.push((Tok::Op(&src[i..i + op_len]), line));
                i += op_len;
            }
        }
    }
    Ok((toks, comment_lines))
}

const fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/// Interpreted string: returns (token text, next index, next line).
fn lex_interpreted_string(b: &[u8], start: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = start + 1;
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

#[expect(
    clippy::too_many_lines,
    reason = "one pass over the token stream that tracks brace depth, the container stack and the open function span together; a split would share about ten mutable state variables through every helper"
)]
fn analyze_go(src: &str) -> Result<Vec<RawFunction>, String> {
    let (toks, comment_lines) = lex(src)?;
    let mut collector = MetricsCollector::new(&GO_SPEC, 0);
    let mut raws = Vec::new();
    let mut i = 0usize;
    while i < toks.len() {
        if let (Tok::Kw(k), ln) = &toks[i]
            && *k == "func"
        {
            let mut j = i + 1;
            let owner = if matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "(") {
                let mut depth = 1;
                j += 1;
                let mut last_ident = String::new();
                while j < toks.len() && depth > 0 {
                    match &toks[j].0 {
                        Tok::Op(o) if *o == "(" => depth += 1,
                        Tok::Op(o) if *o == ")" => depth -= 1,
                        Tok::Ident(n) => last_ident = (*n).to_string(),
                        _ => {}
                    }
                    j += 1;
                }
                last_ident
            } else {
                String::new()
            };
            let name = match (toks.get(j), toks.get(j + 1)) {
                (Some((Tok::Ident(n), _)), Some((Tok::Op(o), _))) if *o == "(" || *o == "[" => {
                    (*n).to_string()
                }
                _ => {
                    i += 1;
                    continue;
                }
            };
            let qualified = if owner.is_empty() {
                name
            } else {
                format!("{owner}::{name}")
            };
            let mut params: Vec<String> = Vec::new();
            let mut pending: Option<String> = None;
            let mut has_result = false;
            let mut depth = 0usize;
            let mut params_done = false;
            while j < toks.len() {
                match &toks[j].0 {
                    Tok::Op(o) if *o == "(" => {
                        if depth == 0 && params_done {
                            has_result = true;
                        }
                        depth += 1;
                    }
                    Tok::Op(o) if *o == ")" => {
                        if depth == 0 {
                            break;
                        }
                        depth -= 1;
                        if depth == 0 && !params_done {
                            params_done = true;
                            pending = None;
                        }
                    }
                    Tok::Op(o) if *o == "{" && depth == 0 => {
                        if !matches!(&toks[j - 1].0, Tok::Kw(k) if *k == "struct" || *k == "interface")
                        {
                            break;
                        }
                    }
                    Tok::Ident(n) if depth == 1 && !params_done => {
                        if let Some(cand) = pending.take() {
                            if !params.contains(&cand) {
                                params.push(cand);
                            }
                        } else {
                            pending = Some((*n).to_string());
                        }
                    }
                    Tok::Op(o) if *o == "," && depth == 1 && !params_done => {
                        if let Some(cand) = pending.take()
                            && !params.contains(&cand)
                        {
                            params.push(cand);
                        }
                    }
                    Tok::Newline if depth == 0 && params_done => break,
                    _ => {
                        if depth == 0 && params_done {
                            has_result = true;
                        }
                    }
                }
                j += 1;
            }
            let io_params = params.len() + usize::from(has_result);
            let ends_at_signature = matches!(toks.get(j), Some((Tok::Newline, _)))
                || (j == toks.len() && params_done && depth == 0);
            if ends_at_signature {
                i = j + 1;
                continue;
            }
            if !matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "{") {
                i = j;
                continue;
            }
            let mut depth = 0usize;
            let mut end_line = *ln;
            let body_start = j + 1;
            let mut k = j;
            while k < toks.len() {
                match &toks[k].0 {
                    Tok::Op(o) if *o == "{" => depth += 1,
                    Tok::Op(o) if *o == "}" => {
                        depth -= 1;
                        if depth == 0 {
                            end_line = toks[k].1;
                            break;
                        }
                    }
                    _ => {}
                }
                k += 1;
            }
            let raw = build_raw(
                &mut collector,
                &toks,
                &comment_lines,
                body_start,
                k,
                *ln,
                end_line,
                &qualified,
                count_u32(io_params),
            );
            raws.push(raw);
            i = k + 1;
            continue;
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
            Tok::Op(o) => match *o {
                "{" => collector.open_block(*ln),
                "}" => collector.close_block(*ln),
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
    let outcome = collector.finish(io_params as usize);
    let span_lines = count_f64(end_line.saturating_sub(def_line) + 1);
    let comment_count = (def_line..=end_line)
        .filter(|l| comment_lines.get(l - 1).copied().unwrap_or(false))
        .count();
    RawFunction {
        path: String::new(),
        language: halmi_core::report::Language::Go,
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

const GO_SPEC: halmi_lex::LangSpec = halmi_lex::LangSpec {
    decl_op: "func",
    keyword_operators: &CONTROL_KEYWORDS,
    cc_keywords: &["if", "for", "case", "select"],
    separator_ops: &["(", ")", "[", "]", "{", "}", ",", ";", ":"],
    cc_ops: &["&&", "||"],
    loc_mode: halmi_lex::LocMode::Physical,
};

#[cfg(test)]
mod kw_tests {
    #[test]
    fn keywords_sorted_for_binary_search() {
        assert!(super::ALL_KEYWORDS.windows(2).all(|w| w[0] <= w[1]));
    }
}
