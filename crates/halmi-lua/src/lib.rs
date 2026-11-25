//! halmi-lua: Lua language adapter.
//!
//! Hand lexer + declaration scanner on [`halmi_lex`] shared machinery.
//! Named `function` declarations count; anonymous `function (`
//! expressions (callbacks/closures) do not. Bodies are `end`-blocks
//! (function/if/do/repeat open; `end`/`until` close; `for`/`while`
//! never need counting because their `do` covers them). Zero
//! dependencies; no tree-sitter (the tree-sitter-lua oracle is a
//! test-side differential only).
//!
//! * Definitions: `function_declaration` parity: `function a.b.c()`,
//!   `function M:method()`, `local function name()`.
//! * `decl_op`: `function`.

#![deny(unsafe_code)]

use halmi_core::adapter::{LanguageAdapter, SourceAnalysis};
use halmi_core::report::{Language, RawFunction, count_f64, count_u32};
use halmi_lex::{LangSpec, LocMode, MetricsCollector};

#[derive(Debug, Clone, Copy, Default)]
pub struct LuaAdapter;

impl LanguageAdapter for LuaAdapter {
    fn language(&self) -> &'static str {
        "Lua"
    }

    fn analyze(&self, path: &str, src: &str) -> SourceAnalysis {
        match analyze_lua(src) {
            Ok(mut raws) => {
                for raw in &mut raws {
                    raw.path = path.to_string();
                    raw.language = Language::Lua;
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

const LUA_SPEC: LangSpec = LangSpec {
    decl_op: "function",
    keyword_operators: CONTROL_KEYWORDS,
    cc_keywords: &["if", "elseif", "while", "for", "and", "or"],
    separator_ops: &["(", ")", "[", "]", "{", "}", ",", ";"],
    cc_ops: &[],
    loc_mode: LocMode::Physical,
};

const CONTROL_KEYWORDS: &[&str] = &[
    "if", "elseif", "else", "while", "for", "repeat", "until", "do", "then", "end", "break",
    "goto", "return", "local", "function", "and", "or", "not", "in",
];

const ALL_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok<'a> {
    Ident(&'a str),
    Number(&'a str),
    Str(&'a str),
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
#[expect(
    clippy::many_single_char_names,
    reason = "the cursor, the current byte and token indices keep their conventional one-letter scanner names (b, c, i, j, k)"
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
            b'-' if i + 1 < b.len() && b[i + 1] == b'-' => {
                let mut j = i + 2;
                if j + 1 < b.len() && b[j] == b'[' {
                    let mut h = 0usize;
                    while j + h < b.len() && b[j + h] == b'=' {
                        h += 1;
                    }
                    if j + h < b.len() && b[j + h] == b'[' {
                        j = j + h + 1;
                        let mut nl = line;
                        let closer = format!("]{}]", "=".repeat(h));
                        while j < b.len() {
                            if src[j..].starts_with(&closer) {
                                j += closer.len();
                                if line >= 1 && line <= n_lines {
                                    comment_lines[line - 1] = true;
                                }
                                for l in line..=nl {
                                    if l >= 1 && l <= n_lines {
                                        comment_lines[l - 1] = true;
                                    }
                                }
                                line = nl;
                                i = j;
                                break;
                            }
                            if b[j] == b'\n' {
                                nl += 1;
                            }
                            j += 1;
                        }
                        if i != j {
                            return Err(format!("line {line}: unterminated long comment"));
                        }
                        continue;
                    }
                }
                if line >= 1 && line <= n_lines {
                    comment_lines[line - 1] = true;
                }
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'"' | b'\'' => {
                let (end, nl) = lex_string(b, i, line)?;
                line = nl;
                toks.push((Tok::Str(&src[i..end]), line));
                i = end;
            }
            b'[' if long_bracket_ahead(src, i) => {
                let (end, nl) = lex_long_string(b, i, line)?;
                line = nl;
                toks.push((Tok::Str(&src[i..end]), line));
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

fn long_bracket_ahead(src: &str, i: usize) -> bool {
    let rest = &src[i..];
    if !rest.starts_with('[') {
        return false;
    }
    let after = &rest[1..];
    let h = after.chars().take_while(|c| *c == '=').count();
    matches!(after.as_bytes().get(h), Some(b'['))
}

fn lex_string(b: &[u8], q: usize, line: usize) -> Result<(usize, usize), String> {
    let quote = b[q];
    let mut i = q + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            c if c == quote => return Ok((i + 1, line)),
            b'\n' => return Err(format!("line {line}: newline in string")),
            _ => i += 1,
        }
    }
    Err(format!("line {line}: unterminated string"))
}

fn lex_long_string(b: &[u8], start: usize, line: usize) -> Result<(usize, usize), String> {
    let mut line = line;
    let mut i = start + 1;
    let mut h = 0usize;
    while i < b.len() && b[i] == b'=' {
        h += 1;
        i += 1;
    }
    if i >= b.len() || b[i] != b'[' {
        return Err(format!("line {line}: bad long string"));
    }
    i += 1;
    if i < b.len() && b[i] == b'\n' {
        line += 1;
        i += 1;
    }
    while i < b.len() {
        if b[i] == b']' {
            let mut h2 = 0usize;
            let mut j = i + 1;
            while j < b.len() && b[j] == b'=' {
                h2 += 1;
                j += 1;
            }
            if h2 == h && j < b.len() && b[j] == b']' {
                return Ok((j + 1, line));
            }
        }
        if b[i] == b'\n' {
            line += 1;
        }
        i += 1;
    }
    Err(format!("line {line}: unterminated long string"))
}

const fn lex_number(b: &[u8], start: usize) -> usize {
    let mut i = start;
    if b[i] == b'0' && i + 1 < b.len() && matches!(b[i + 1], b'x' | b'X') {
        i += 2;
        while i < b.len() && b[i].is_ascii_hexdigit() {
            i += 1;
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
    i
}

/// Index of the matching `end`/`until` for the block whose opener is
/// at `open` (function/if/do/repeat `{`-free Lua blocks).
fn find_end(toks: &[(Tok<'_>, usize)], open: usize) -> Option<usize> {
    let mut stack: Vec<usize> = Vec::new();
    let mut k = open;
    while k < toks.len() {
        if let Tok::Kw(kw) = &toks[k].0 {
            match &**kw {
                "function" | "if" | "do" | "repeat" => stack.push(k),
                "until" | "end" => {
                    stack.pop();
                    if stack.is_empty() {
                        return Some(k);
                    }
                }
                _ => {}
            }
        }
        k += 1;
    }
    None
}

fn analyze_lua(src: &str) -> Result<Vec<RawFunction>, String> {
    let (toks, comment_lines) = lex(src)?;
    let mut collector = MetricsCollector::new(&LUA_SPEC, 0);
    let mut raws = Vec::new();
    let mut i = 0usize;
    while i < toks.len() {
        match &toks[i].0 {
            Tok::Kw(k) if *k == "function" => {
                let first = toks.get(i + 1).map(|t| &t.0);
                if matches!(first, Some(Tok::Op(o)) if *o == "(") {
                    i += 1;
                    continue;
                }
                let Some(Tok::Ident(first_name)) = first else {
                    i += 1;
                    continue;
                };
                let ln = toks[i].1;
                let mut name = (*first_name).to_string();
                let mut j = i + 2;
                while matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "." || *o == ":") {
                    if let Some((Tok::Ident(n), _)) = toks.get(j + 1) {
                        let sep = match &toks[j].0 {
                            Tok::Op(o) if *o == "." => '.',
                            _ => ':',
                        };
                        name.push(sep);
                        name.push_str(n);
                        j += 2;
                    } else {
                        break;
                    }
                }
                if !matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "(") {
                    i += 1;
                    continue;
                }
                let mut params: Vec<String> = Vec::new();
                let mut pdepth = 1usize;
                j += 1;
                while j < toks.len() && pdepth > 0 {
                    match &toks[j].0 {
                        Tok::Op(o) if *o == "(" => pdepth += 1,
                        Tok::Op(o) if *o == ")" => pdepth -= 1,
                        Tok::Ident(n) if pdepth == 1 => {
                            let n = (*n).to_string();
                            if !params.contains(&n) {
                                params.push(n);
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if pdepth != 0 {
                    i += 1;
                    continue;
                }
                if params.len() == 1 && (params[0] == "self" || params[0] == "...") {
                    params.clear();
                }
                let Some(end_idx) = find_end(&toks, i) else {
                    return Err(format!("line {ln}: unterminated function `{name}`"));
                };
                let span_start = if i > 0 && matches!(&toks[i - 1].0, Tok::Kw(p) if *p == "local") {
                    i - 1
                } else {
                    i
                };
                let decl_line = toks[span_start].1;
                let end_line = toks[end_idx].1;
                let io_params = count_u32(params.len() + 1);
                let raw = build_raw(
                    &mut collector,
                    &toks,
                    &comment_lines,
                    i + 1,
                    end_idx,
                    decl_line,
                    end_line,
                    &name,
                    io_params,
                );
                raws.push(raw);
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
            Tok::Number(n) | Tok::Str(n) => {
                collector.token(&halmi_lex::Tok::Str(n), *ln);
            }
            Tok::Kw(k) => collector.token(&halmi_lex::Tok::Keyword(k), *ln),
            Tok::Op(o) => match *o {
                "{" => collector.open_block(*ln),
                "}" => collector.close_block(*ln),
                "(" => {
                    let callee = toks.get(idx.wrapping_sub(1)).map(|t| &t.0);
                    let is_method = matches!(toks.get(idx.wrapping_sub(2)), Some((Tok::Op(p), _)) if *p == "." || *p == ":");
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
        language: Language::Lua,
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
