//! halmi-php: PHP language adapter.
//!
//! Hand lexer + declaration scanner on [`halmi_lex`] shared machinery.
//! The `function` keyword triggers candidates (anonymous `function (`
//! closures and `fn` arrows do not count). Zero dependencies, no
//! tree-sitter: the tree-sitter-php oracle is a test-side
//! differential only.
//!
//! * Definitions: `function_definition` / `method_declaration`
//!   parity: top-level functions, class/trait/interface/enum methods
//!   (qualified `Widget::area`), constructors (`__construct`), and
//!   bodyless `abstract`/`interface` methods. Modifier and
//!   `#[Attribute]` runs fold into the span.
//! * `decl_op`: `function`.

#![deny(unsafe_code)]

use halmi_core::adapter::{LanguageAdapter, SourceAnalysis};
use halmi_core::report::{Language, RawFunction, count_f64, count_u32};
use halmi_lex::{LangSpec, LocMode, MetricsCollector};

#[derive(Debug, Clone, Copy, Default)]
pub struct PhpAdapter;

impl LanguageAdapter for PhpAdapter {
    fn language(&self) -> &'static str {
        "Php"
    }

    fn analyze(&self, path: &str, src: &str) -> SourceAnalysis {
        match analyze_php(src) {
            Ok(mut raws) => {
                for raw in &mut raws {
                    raw.path = path.to_string();
                    raw.language = Language::Php;
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

const PHP_SPEC: LangSpec = LangSpec {
    decl_op: "function",
    keyword_operators: CONTROL_KEYWORDS,
    cc_keywords: &["if", "elseif", "for", "foreach", "while", "case", "catch"],
    separator_ops: &["(", ")", "[", "]", "{", "}", ",", ";", ":"],
    cc_ops: &["&&", "||"],
    loc_mode: LocMode::Physical,
};

const CONTROL_KEYWORDS: &[&str] = &[
    "if",
    "else",
    "elseif",
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
    "clone",
    "echo",
    "print",
    "instanceof",
    "unset",
    "isset",
    "empty",
    "list",
    "include",
    "require",
    "yield",
    "match",
    "arrow",
];

const ALL_KEYWORDS: &[&str] = &[
    "abstract",
    "and",
    "array",
    "as",
    "break",
    "callable",
    "case",
    "catch",
    "class",
    "clone",
    "const",
    "continue",
    "declare",
    "default",
    "do",
    "echo",
    "else",
    "elseif",
    "empty",
    "enddeclare",
    "endfor",
    "endforeach",
    "endif",
    "endswitch",
    "endwhile",
    "enum",
    "extends",
    "final",
    "finally",
    "fn",
    "for",
    "foreach",
    "function",
    "global",
    "goto",
    "if",
    "implements",
    "include",
    "include_once",
    "instanceof",
    "insteadof",
    "interface",
    "isset",
    "list",
    "match",
    "namespace",
    "new",
    "or",
    "print",
    "private",
    "protected",
    "public",
    "readonly",
    "require",
    "require_once",
    "return",
    "static",
    "switch",
    "throw",
    "trait",
    "try",
    "unset",
    "use",
    "var",
    "while",
    "xor",
    "yield",
];

const MODIFIERS: &[&str] = &[
    "public",
    "private",
    "protected",
    "static",
    "abstract",
    "final",
    "readonly",
];

const CONTAINER_KEYWORDS: &[&str] = &["class", "interface", "trait", "enum"];

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
            b'#' if i + 1 >= b.len() || b[i + 1] != b'[' => {
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
                let (end, nl) = lex_dq(b, i, line)?;
                line = nl;
                toks.push((Tok::Str(&src[i..end]), line));
                i = end;
            }
            b'\'' => {
                let (end, nl) = lex_sq(b, i, line)?;
                line = nl;
                toks.push((Tok::Str(&src[i..end]), line));
                i = end;
            }
            b'<' if src[i..].starts_with("<<<") => {
                let (end, nl) = lex_heredoc(b, i, line)?;
                line = nl;
                toks.push((Tok::Str(&src[i..end]), line));
                i = end;
            }
            b'$' if i + 1 < b.len() && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_') => {
                let start = i;
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                toks.push((Tok::Ident(&src[start..i]), line));
            }
            b'?' if i + 1 < b.len() && b[i + 1] == b'?' && src[i..].starts_with("?->") => {
                toks.push((Tok::Op("?->"), line));
                i += 3;
            }
            c if c.is_ascii_digit() => {
                let end = lex_number(b, i);
                toks.push((Tok::Number(&src[i..end]), line));
                i = end;
            }
            c if c.is_ascii_alphabetic() || c == b'\\' || c == b'_' => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'\\')
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

fn lex_dq(b: &[u8], q: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = q + 1;
    let mut line = line;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'{' if i + 1 < b.len() && b[i + 1] == b'$' => {
                let mut d = 0usize;
                while i < b.len() {
                    if b[i] == b'{' {
                        d += 1;
                    } else if b[i] == b'}' {
                        d -= 1;
                        if d == 0 {
                            break;
                        }
                    }
                    i += 1;
                }
                i += 1;
            }
            b'"' => return Ok((i + 1, line)),
            b'\n' => {
                line += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    Err(format!("line {line}: unterminated string"))
}

fn lex_sq(b: &[u8], q: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = q + 1;
    let mut line = line;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'\'' => return Ok((i + 1, line)),
            b'\n' => {
                line += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    Err(format!("line {line}: unterminated string"))
}

fn lex_heredoc(b: &[u8], start: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = start + 3;
    while i < b.len() && (b[i] == b'-' || b[i] == b'~') {
        i += 1;
    }
    let quoted = i < b.len() && (b[i] == b'\'' || b[i] == b'"');
    let quote = if quoted { b[i] } else { 0 };
    if quoted {
        i += 1;
    }
    let id_start = i;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
        i += 1;
    }
    let id = &b[id_start..i];
    if quoted && i < b.len() && b[i] == quote {
        i += 1;
    }
    while i < b.len() && b[i] != b'\n' {
        i += 1;
    }
    let mut line = line;
    while i < b.len() {
        if b[i] == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        let mut j = i;
        let mut only_ws = true;
        while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
            j += 1;
        }
        let id_pos = j;
        if b[j..].starts_with(id)
            && (j + id.len() >= b.len()
                || b[j + id.len()] == b';'
                || b[j + id.len()] == b'\n'
                || b[j + id.len()] == b'\r')
        {
            let mut e = j + id.len();
            if e < b.len() && b[e] == b';' {
                e += 1;
            }
            let _ = only_ws;
            let _ = id_pos;
            return Ok((e, line));
        }
        only_ws = false;
        let _ = only_ws;
        while i < b.len() && b[i] != b'\n' {
            i += 1;
        }
    }
    Err(format!("line {line}: unterminated heredoc"))
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
    i
}

fn skip_back_brackets(toks: &[(Tok<'_>, usize)], close: usize) -> usize {
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

/// What the tokens before a `{` say about it: not a container body, or a
/// container body whose name may be anonymous (`enum { ... }`, `companion object`).
enum Container {
    Not,
    Body(Option<String>),
}

fn container_name_for(toks: &[(Tok<'_>, usize)], brace: usize) -> Container {
    let mut j = brace;
    if j == 0 {
        return Container::Not;
    }
    j -= 1;
    if matches!(&toks[j].0, Tok::Op(o) if *o == "]") {
        let open = skip_back_brackets(toks, j);
        if open == 0 || open > j {
            return Container::Not;
        }
        j = open.saturating_sub(1);
    }
    if matches!(&toks[j].0, Tok::Op(o) if *o == ")") {
        return Container::Body(None);
    }
    let mut name: Option<String> = None;
    if let Tok::Ident(n) = &toks[j].0 {
        name = Some((*n).to_string());
        if j == 0 {
            return Container::Not;
        }
        j -= 1;
        if matches!(&toks[j].0, Tok::Op(o) if *o == "," || *o == "{") {
            return Container::Body(name);
        }
    }
    while j > 0 {
        match &toks[j].0 {
            Tok::Newline | Tok::Ident(_) => j -= 1,
            Tok::Kw(k) if matches!(&**k, "extends" | "implements") => j -= 1,
            Tok::Op(o) if matches!(&**o, "," | "|" | "&") => j -= 1,
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

fn function_head(toks: &[(Tok<'_>, usize)], i: usize) -> Option<(String, Vec<String>, usize)> {
    let mut j = i + 1;
    if matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "(") {
        return None;
    }
    if matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "&") {
        j += 1;
    }
    let name = match toks.get(j).map(|t| &t.0) {
        Some(Tok::Ident(n) | Tok::Kw(n)) => (*n).to_string(),
        _ => return None,
    };
    j += 1;
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

fn function_terminator(toks: &[(Tok<'_>, usize)], mut k: usize) -> Option<(usize, bool)> {
    while k < toks.len() {
        match &toks[k].0 {
            Tok::Op(o) if *o == "{" => return Some((k, true)),
            Tok::Op(o) if *o == ";" => return Some((k, false)),
            Tok::Newline | Tok::Ident(_) | Tok::Kw(_) => k += 1,
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
            Tok::Op(o) if matches!(&**o, ":" | "?" | "|" | "&" | "." | "..." | "\\") => k += 1,
            _ => break,
        }
    }
    None
}

fn declaration_start(toks: &[(Tok<'_>, usize)], i: usize) -> usize {
    let mut span_start = i;
    let mut w = i;
    let mut crossed = false;
    while w > 0 {
        let prev = w - 1;
        match &toks[prev].0 {
            Tok::Op(o) if *o == "]" => {
                let open = skip_back_brackets(toks, prev);
                if open > 0 && matches!(&toks[open.wrapping_sub(1)].0, Tok::Op(p) if *p == "#") {
                    span_start = open - 1;
                    w = open - 1;
                    crossed = false;
                } else if open > 0 {
                    span_start = open;
                    w = open;
                    crossed = false;
                } else {
                    break;
                }
            }
            Tok::Newline
                if !crossed
                    && w >= 2
                    && (matches!(&toks[prev - 1].0, Tok::Kw(p) if MODIFIERS.contains(&&**p))
                        || matches!(&toks[prev - 1].0, Tok::Op(p) if *p == "]" || *p == "#")) =>
            {
                crossed = true;
                span_start = prev - 1;
                w = prev - 1;
            }
            Tok::Kw(p) if MODIFIERS.contains(&&**p) => {
                span_start = prev;
                w = prev;
            }
            _ => break,
        }
    }
    span_start
}

fn php_body_bounds(
    toks: &[(Tok<'_>, usize)],
    term: usize,
    has_body: bool,
) -> (usize, usize, usize) {
    if !has_body {
        return (term, term, toks[term].1);
    }
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
}

fn analyze_php(src: &str) -> Result<Vec<RawFunction>, String> {
    let (toks, comment_lines) = lex(src)?;
    let mut collector = MetricsCollector::new(&PHP_SPEC, 0);
    let mut raws = Vec::new();
    let mut containers: Vec<(Option<String>, usize)> = Vec::new();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < toks.len() {
        match &toks[i].0 {
            Tok::Kw(k) if *k == "function" => {
                let Some((name, params, j)) = function_head(&toks, i) else {
                    i += 1;
                    continue;
                };
                let owner = containers
                    .last()
                    .and_then(|(n, _)| n.clone())
                    .unwrap_or_default();
                let qualified = if owner.is_empty() {
                    name.clone()
                } else {
                    format!("{owner}::{name}")
                };
                let Some((term, has_body)) = function_terminator(&toks, j) else {
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
                let is_ctor = name == "__construct";
                let has_result = !is_ctor
                    && !(ret_words.len() == 1
                        && (ret_words[0] == "void" || ret_words[0] == "never"));
                let io_params = count_u32(params.len() + usize::from(has_result));
                let (body_start, body_end, end_line) = php_body_bounds(&toks, term, has_body);
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
                if has_body && body_end > i {
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
            Tok::Number(n) | Tok::Str(n) => {
                collector.token(&halmi_lex::Tok::Str(n), *ln);
            }
            Tok::Kw(k) => collector.token(&halmi_lex::Tok::Keyword(k), *ln),
            Tok::Op(o) => match *o {
                "{" => collector.open_block(*ln),
                "}" => collector.close_block(*ln),
                "(" => {
                    let callee = toks.get(idx.wrapping_sub(1)).map(|t| &t.0);
                    let is_method = matches!(toks.get(idx.wrapping_sub(2)), Some((Tok::Op(p), _)) if *p == "->" || *p == "?->" || *p == "::");
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
        language: Language::Php,
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
