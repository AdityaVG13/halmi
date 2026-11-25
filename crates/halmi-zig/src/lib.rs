//! halmi-zig: Zig language adapter.
//!
//! Hand lexer + declaration scanner on [`halmi_lex`] shared machinery.
//! Zero dependencies, no tree-sitter: the tree-sitter-zig oracle is a
//! test-side differential only.
//!
//! * Declarations: `fn` (with `pub`/`export`/`extern`/`inline`/
//!   `noinline`/`callconv(...)`/`linksection(...)`/`align(...)` modifier
//!   runs folded into the span) and `test` blocks. A `fn` in type
//!   position (`*const fn (u32) u32`) is not a declaration. Bodyless
//!   `extern fn ...;` declarations count.
//! * Qualification: `Container.name` where the container is the
//!   innermost `const NAME = struct|enum|union|error|opaque { ... }`.
//! * Operands: identifiers (including `@builtin`s, `@"raw idents"`),
//!   numbers, strings (regular and `\\` multiline), chars.
//! * Cyclomatic complexity: 1 + one per `if`, `while`, `for`, `catch`,
//!   `and`, `or`.
//! * Physical LOC; nesting = brace depth inside the body (body = 0).
//! * `io_params`: distinct parameter names + 1 when the return type is
//!   not exactly `void`.

#![deny(unsafe_code)]

use halmi_core::adapter::{LanguageAdapter, SourceAnalysis};
use halmi_core::report::Language;
use halmi_core::report::{RawFunction, count_f64, count_u32};
use halmi_lex::{LangSpec, LocMode, MetricsCollector};

#[derive(Debug, Clone, Copy, Default)]
pub struct ZigAdapter;

impl LanguageAdapter for ZigAdapter {
    fn language(&self) -> &'static str {
        "Zig"
    }

    fn analyze(&self, path: &str, src: &str) -> SourceAnalysis {
        match analyze_zig(src) {
            Ok(mut raws) => {
                for raw in &mut raws {
                    raw.path = path.to_string();
                    raw.language = Language::Zig;
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

const ZIG_SPEC: LangSpec = LangSpec {
    decl_op: "fn",
    keyword_operators: CONTROL_KEYWORDS,
    cc_keywords: &["if", "while", "for", "catch", "and", "or"],
    separator_ops: &["(", ")", "[", "]", "{", "}", ",", ";", ":"],
    cc_ops: &[],
    loc_mode: LocMode::Physical,
};

const CONTROL_KEYWORDS: &[&str] = &[
    "fn",
    "if",
    "else",
    "while",
    "for",
    "switch",
    "return",
    "break",
    "continue",
    "defer",
    "errdefer",
    "try",
    "catch",
    "orelse",
    "and",
    "or",
    "suspend",
    "resume",
    "await",
    "unreachable",
];

const ALL_KEYWORDS: &[&str] = &[
    "addrspace",
    "align",
    "allowzero",
    "and",
    "anyframe",
    "anytype",
    "asm",
    "async",
    "await",
    "break",
    "callconv",
    "catch",
    "comptime",
    "const",
    "continue",
    "defer",
    "else",
    "enum",
    "errdefer",
    "error",
    "export",
    "extern",
    "fn",
    "for",
    "if",
    "inline",
    "linksection",
    "noinline",
    "nosuspend",
    "opaque",
    "or",
    "orelse",
    "packed",
    "pub",
    "resume",
    "return",
    "struct",
    "suspend",
    "switch",
    "test",
    "threadlocal",
    "try",
    "union",
    "unreachable",
    "usingnamespace",
    "var",
    "volatile",
    "while",
];

const MODIFIERS: &[&str] = &[
    "pub",
    "export",
    "extern",
    "inline",
    "noinline",
    "threadlocal",
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
            b'@' if i + 1 < b.len() && b[i + 1] == b'"' => {
                let (end, nl) = lex_raw_ident(b, i + 1, line)?;
                line = nl;
                toks.push((Tok::Ident(&src[i + 2..end - 1]), line));
                i = end;
            }
            b'@' if i + 1 < b.len() && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_') => {
                let start = i + 1;
                let mut j = start;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                toks.push((Tok::Ident(&src[start..j]), line));
                i = j;
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
            b'\\' if i + 1 < b.len() && b[i + 1] == b'\\' => {
                let start = i;
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                toks.push((Tok::Str(&src[start..i]), line));
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

fn lex_raw_ident(b: &[u8], q: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = q + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Ok((i + 1, line)),
            b'\n' => return Err(format!("line {line}: newline in raw identifier")),
            _ => i += 1,
        }
    }
    Err(format!("line {line}: unterminated raw identifier"))
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
            b'\n' => return Err(format!("line {line}: newline in character literal")),
            _ => i += 1,
        }
    }
    Err(format!("line {line}: unterminated character literal"))
}

const fn lex_number(b: &[u8], start: usize) -> usize {
    let mut i = start;
    if b[i] == b'0' && i + 1 < b.len() && matches!(b[i + 1], b'x' | b'o' | b'b') {
        i += 2;
        while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
            i += 1;
        }
        if i < b.len() && b[i] == b'.' && i + 1 < b.len() && (b[i + 1] as char).is_ascii_hexdigit()
        {
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
        }
        return i;
    }
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
    i
}

fn at_member_boundary(toks: &[(Tok<'_>, usize)], j: usize) -> bool {
    match toks.get(j.wrapping_sub(1)) {
        None | Some((Tok::Newline, _)) => true,
        Some((Tok::Op(o), _)) => matches!(&**o, "{" | "}" | ";"),
        _ => false,
    }
}

fn is_type_container(k: &str) -> bool {
    matches!(k, "struct" | "enum" | "union" | "error" | "opaque")
}

#[expect(
    clippy::too_many_lines,
    reason = "one pass over the token stream that tracks brace depth, the container stack and the open function span together; a split would share about ten mutable state variables through every helper"
)]
#[expect(
    clippy::many_single_char_names,
    reason = "the cursor, the current byte and token indices keep their conventional one-letter scanner names (b, c, i, j, k)"
)]
fn analyze_zig(src: &str) -> Result<Vec<RawFunction>, String> {
    let (toks, comment_lines) = lex(src)?;
    let mut collector = MetricsCollector::new(&ZIG_SPEC, 0);
    let mut raws = Vec::new();
    let mut containers: Vec<(String, usize)> = Vec::new();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < toks.len() {
        match &toks[i].0 {
            Tok::Kw(k) if *k == "fn" => {
                if !matches!(toks.get(i + 1), Some((Tok::Ident(_), _))) {
                    i += 1;
                    continue;
                }
                let span_start = modifier_run_start(&toks, i);
                let decl_line = toks[span_start].1;
                let name = match &toks[i + 1].0 {
                    Tok::Ident(n) => (*n).to_string(),
                    _ => unreachable!(),
                };
                let owner = containers.last().map_or("", |c| c.0.as_str());
                let qualified = if owner.is_empty() {
                    name.clone()
                } else {
                    format!("{owner}::{name}")
                };
                let mut j = i + 2;
                let mut params: Vec<String> = Vec::new();
                let mut pending: Option<String> = None;
                let mut pdepth = 0usize;
                let mut params_done = false;
                let mut ret_toks: Vec<&Tok<'_>> = Vec::new();
                let mut rdepth = 0usize;
                let mut body_idx: Option<usize> = None;
                let mut decl_end = None;
                while j < toks.len() {
                    match &toks[j].0 {
                        Tok::Op(o) if *o == "(" => {
                            pdepth += 1;
                            if params_done {
                                ret_toks.push(&toks[j].0);
                            }
                        }
                        Tok::Op(o) if *o == ")" => {
                            pdepth = pdepth.saturating_sub(1);
                            if pdepth == 0 && !params_done {
                                params_done = true;
                                pending = None;
                            } else if params_done {
                                ret_toks.push(&toks[j].0);
                            }
                        }
                        Tok::Op(o) if *o == "{" && pdepth == 0 && params_done => {
                            let container = matches!(
                                toks.get(j.wrapping_sub(1)).map(|t| &t.0),
                                Some(Tok::Kw(p)) if is_type_container(p)
                            );
                            if (container && rdepth == 0) || rdepth > 0 {
                                rdepth += 1;
                            } else {
                                body_idx = Some(j);
                                break;
                            }
                        }
                        Tok::Op(o) if *o == "}" && pdepth == 0 && params_done && rdepth > 0 => {
                            rdepth -= 1;
                        }
                        Tok::Op(o) if *o == ";" && pdepth == 0 && params_done && rdepth == 0 => {
                            decl_end = Some(j);
                            break;
                        }
                        Tok::Ident(n) if pdepth == 1 && !params_done => {
                            if let Some(cand) = pending.take() {
                                if !params.contains(&cand) {
                                    params.push(cand);
                                }
                            } else {
                                pending = Some((*n).to_string());
                            }
                        }
                        Tok::Op(o) if *o == "," && pdepth == 1 && !params_done => {
                            if let Some(cand) = pending.take()
                                && !params.contains(&cand)
                            {
                                params.push(cand);
                            }
                        }
                        other => {
                            if params_done && pdepth == 0 {
                                ret_toks.push(other);
                            }
                        }
                    }
                    j += 1;
                }
                let has_result =
                    !(ret_toks.len() == 1 && matches!(ret_toks[0], Tok::Ident(v) if *v == "void"));
                let io_params =
                    count_u32(params.len() + usize::from(!ret_toks.is_empty() && has_result));
                let (body_start, body_end, end_line) = match (body_idx, decl_end) {
                    (Some(bi), _) => {
                        let be = find_body_end(&toks, bi);
                        (bi + 1, be, toks[be.min(toks.len().saturating_sub(1))].1)
                    }
                    (None, Some(se)) => (se, se, toks[se].1),
                    (None, None) => {
                        return Err(format!(
                            "line {decl_line}: function `{name}` without body terminator"
                        ));
                    }
                };
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
                i += 1;
            }
            Tok::Kw(k) if *k == "test" && at_member_boundary(&toks, i) => {
                let mut j = i + 1;
                let mut tname = "test".to_string();
                match toks.get(j) {
                    Some((Tok::Str(s), _)) => {
                        let inner = s.trim_matches('"');
                        tname = format!("test \"{inner}\"");
                        j += 1;
                    }
                    Some((Tok::Ident(n), _)) => {
                        tname = format!("test {n}");
                        j += 1;
                    }
                    _ => {}
                }
                if !matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "{") {
                    i += 1;
                    continue;
                }
                let decl_line = toks[i].1;
                let mut bd = 0usize;
                let mut k = j;
                let mut el = toks[j].1;
                while k < toks.len() {
                    match &toks[k].0 {
                        Tok::Op(o) if *o == "{" => bd += 1,
                        Tok::Op(o) if *o == "}" => {
                            bd = bd.saturating_sub(1);
                            if bd == 0 {
                                el = toks[k].1;
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
                    j + 1,
                    k,
                    decl_line,
                    el,
                    &tname,
                    0,
                );
                raws.push(raw);
                i += 1;
            }
            Tok::Op(o) if *o == "{" => {
                let container = container_name_for(&toks, i);
                depth += 1;
                if let Some(name) = container {
                    containers.push((name, depth));
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

/// A `{` opens a container scope when the preceding tokens read
/// `NAME = struct|enum|union|error|opaque` (with `pub` allowed).
fn container_name_for(toks: &[(Tok<'_>, usize)], brace: usize) -> Option<String> {
    let mut j = brace;
    if j == 0 {
        return None;
    }
    j -= 1;
    match &toks[j].0 {
        Tok::Kw(k) if is_type_container(k) => {}
        _ => return None,
    }
    if j == 0 {
        return None;
    }
    j -= 1;
    match &toks[j].0 {
        Tok::Op(o) if *o == "=" => {}
        _ => return None,
    }
    if j == 0 {
        return None;
    }
    j -= 1;
    match &toks[j].0 {
        Tok::Ident(n) => Some((*n).to_string()),
        _ => None,
    }
}

/// Walk back over a modifier run (`pub`, `export`, `extern`, `inline`,
/// `noinline`, `threadlocal`, `callconv(...)`, `linksection(...)`,
/// `align(...)`) that starts at a member boundary. Returns the token
/// index where the declaration span starts.
fn modifier_run_start(toks: &[(Tok<'_>, usize)], fn_idx: usize) -> usize {
    let mut start = fn_idx;
    let mut j = fn_idx;
    while j > 0 {
        let prev = j - 1;
        match &toks[prev].0 {
            Tok::Newline => {
                j = prev;
            }
            Tok::Kw(k) if MODIFIERS.contains(&&**k) => {
                start = prev;
                j = prev;
            }
            Tok::Op(o) if *o == ")" => {
                let open = skip_back_parens(toks, prev);
                if open == 0 {
                    break;
                }
                let before = open - 1;
                let modifier = match &toks[before].0 {
                    Tok::Ident(n) => Some(&**n),
                    Tok::Kw(k) => Some(&**k),
                    _ => None,
                };
                match modifier {
                    Some("callconv" | "linksection" | "align") => {
                        start = before;
                        j = before;
                    }
                    _ => break,
                }
            }
            _ => break,
        }
    }
    if start == fn_idx || at_member_boundary(toks, start) {
        start
    } else {
        fn_idx
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

fn find_body_end(toks: &[(Tok<'_>, usize)], opener: usize) -> usize {
    let mut depth = 0usize;
    let mut k = opener;
    while k < toks.len() {
        match &toks[k].0 {
            Tok::Op(o) if *o == "{" => depth += 1,
            Tok::Op(o) if *o == "}" => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return k;
                }
            }
            _ => {}
        }
        k += 1;
    }
    toks.len()
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
        language: Language::Zig,
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
