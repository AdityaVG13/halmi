//! halmi-swift: the Swift adapter for halmi.
//!
//! Hand-written lexer, no parser dependency. One [`RawFunction`] per
//! `func`, `init`, and `deinit` declaration (methods and initializers
//! included, accessors and closures excluded, matching tree-sitter-swift
//! `function_declaration`/`init_declaration`/`deinit_declaration` spans).
//! Methods are qualified `Type.name`. Contract: the shared
//! [`LanguageAdapter`] port (pure, never panics, degraded files become
//! error entries).
//!
//! Classification (token-faithful; see CONTRACT.md):
//! * Operators: Swift symbol operators via the shared C-family
//!   punctuator table (incl. `?.`, `??`, `..<`, `...`, `->`), `call`,
//!   `index`, and the control/declaration keywords listed in
//!   [`KEYWORD_OPERATORS`]. Modifiers (`public`, `static`, ...) are not
//!   operators.
//! * Operands: identifiers (incl. `self`, `nil`, `true`, `false`, `$0`),
//!   numbers, and strings (regular, multiline `"""`, raw `#"..."#`, all
//!   with `\(expr)` interpolation collapsed into the literal).
//! * Cyclomatic complexity: 1 + one per `if`, `guard`, `for`, `while`,
//!   `case`, `&&`, `||`. (`repeat` is covered by its trailing `while`.)
//! * Physical LOC: distinct lines carrying body tokens (header and
//!   final brace excluded, like the Go adapter).
//! * Nesting: brace depth inside the body (body = 0).
//! * `io_params`: parameter count plus 1 when a `->` result type is
//!   present. `method_chain`: consecutive calls whose receiver is the
//!   result of the previous call.

#![deny(unsafe_code)]

use halmi_core::adapter::{LanguageAdapter, SourceAnalysis};
use halmi_core::report::{RawFunction, count_f64, count_u32};
use halmi_lex::MetricsCollector;
use halmi_lex::cfamily;

/// The Swift adapter.
#[derive(Debug, Clone, Copy, Default)]
pub struct SwiftAdapter;

const KEYWORD_OPERATORS: [&str; 35] = [
    "func",
    "init",
    "deinit",
    "if",
    "else",
    "guard",
    "for",
    "while",
    "repeat",
    "switch",
    "case",
    "default",
    "return",
    "break",
    "continue",
    "fallthrough",
    "do",
    "catch",
    "throw",
    "throws",
    "rethrows",
    "try",
    "defer",
    "as",
    "is",
    "in",
    "where",
    "await",
    "async",
    "class",
    "struct",
    "enum",
    "protocol",
    "extension",
    "actor",
];

const ALL_KEYWORDS: [&str; 60] = [
    "actor",
    "as",
    "associatedtype",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "continue",
    "convenience",
    "default",
    "defer",
    "deinit",
    "do",
    "else",
    "enum",
    "extension",
    "fallthrough",
    "fileprivate",
    "final",
    "for",
    "func",
    "guard",
    "if",
    "import",
    "in",
    "indirect",
    "init",
    "internal",
    "is",
    "lazy",
    "let",
    "mutating",
    "nonmutating",
    "open",
    "operator",
    "optional",
    "override",
    "precedencegroup",
    "private",
    "protocol",
    "public",
    "repeat",
    "required",
    "rethrows",
    "return",
    "static",
    "struct",
    "subscript",
    "switch",
    "throw",
    "throws",
    "try",
    "typealias",
    "unowned",
    "var",
    "weak",
    "where",
    "while",
];

const MODIFIERS: [&str; 17] = [
    "static",
    "final",
    "override",
    "public",
    "private",
    "internal",
    "fileprivate",
    "open",
    "required",
    "convenience",
    "lazy",
    "weak",
    "unowned",
    "mutating",
    "nonmutating",
    "optional",
    "indirect",
];

const TYPE_KEYWORDS: [&str; 6] = ["class", "struct", "enum", "protocol", "extension", "actor"];

impl LanguageAdapter for SwiftAdapter {
    fn language(&self) -> &'static str {
        "Swift"
    }

    fn analyze(&self, path: &str, src: &str) -> SourceAnalysis {
        match analyze_swift(src) {
            Ok(mut raws) => {
                for raw in &mut raws {
                    raw.path = path.to_string();
                    raw.language = halmi_core::report::Language::Swift;
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
}

/// A `/` opens a regex literal where an expression can start: after an
/// operator/punctuator or a keyword, or at the top of the file. After
/// an identifier or literal it is division.
fn regex_context(toks: &[(Tok<'_>, usize)]) -> bool {
    let mut j = toks.len();
    while j > 0 {
        j -= 1;
        match &toks[j].0 {
            Tok::Newline => {}
            Tok::Op(o) => return !matches!(&**o, ")" | "]" | "}" | "." | "?."),
            Tok::Kw(_) => return true,
            _ => return false,
        }
    }
    true
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
                if line <= n_lines {
                    comment_lines[line - 1] = true;
                }
                if !cfamily::skip_line_comment(b, &mut i) {
                    break;
                }
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'*' => {
                if line <= n_lines {
                    comment_lines[line - 1] = true;
                }
                cfamily::skip_block_comment(b, &mut i, &mut line, n_lines, &mut comment_lines)?;
            }
            b'"' => {
                let (end, nl) = if src[i..].starts_with("\"\"\"") {
                    scan_multiline(b, i, line)?
                } else {
                    scan_str(b, i, line)?
                };
                toks.push((Tok::Str(&src[i..end]), line));
                line = nl;
                i = end;
            }
            b'#' if {
                let mut h = 0usize;
                while i + h < b.len() && b[i + h] == b'#' {
                    h += 1;
                }
                i + h < b.len() && b[i + h] == b'"'
            } =>
            {
                let mut hashes = 0usize;
                while i + hashes < b.len() && b[i + hashes] == b'#' {
                    hashes += 1;
                }
                let q = i + hashes;
                let (end, nl) = if src[q..].starts_with("\"\"\"") {
                    scan_multiline(b, q, line)?
                } else {
                    scan_raw(b, q, hashes, line)?
                };
                toks.push((Tok::Str(&src[i..end]), line));
                line = nl;
                i = end;
            }
            b'#' if {
                let mut h = 0usize;
                while i + h < b.len() && b[i + h] == b'#' {
                    h += 1;
                }
                i + h < b.len() && b[i + h] == b'/'
            } =>
            {
                let mut hashes = 0usize;
                while i + hashes < b.len() && b[i + hashes] == b'#' {
                    hashes += 1;
                }
                let (end, nl) = scan_hash_regex(b, i, hashes, line)?;
                toks.push((Tok::Str(&src[i..end]), line));
                line = nl;
                i = end;
            }
            b'#' if i + 1 < b.len() && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_') => {
                let start = i;
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                toks.push((Tok::Ident(&src[start..i]), line));
            }
            b'\\' => {
                let start = i;
                i += 1;
                while i < b.len()
                    && (b[i].is_ascii_alphanumeric()
                        || b[i] == b'_'
                        || b[i] == b'.'
                        || b[i] == b'$')
                {
                    i += 1;
                }
                toks.push((Tok::Ident(&src[start..i]), line));
            }
            b'`' => {
                let start = i;
                i += 1;
                while i < b.len() && b[i] != b'`' && b[i] != b'\n' {
                    i += 1;
                }
                if i >= b.len() || b[i] != b'`' {
                    return Err(format!("line {line}: unterminated backtick identifier"));
                }
                i += 1;
                toks.push((Tok::Ident(&src[start + 1..i - 1]), line));
            }
            b'0'..=b'9' => {
                let start = i;
                i = cfamily::scan_number(b, i);
                toks.push((Tok::Number(&src[start..i]), line));
            }
            b'_' | b'$' | b'a'..=b'z' | b'A'..=b'Z' => {
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
                if !c.is_ascii() {
                    let len = cfamily::utf8_len(c);
                    toks.push((Tok::Op(&src[i..i + len]), line));
                    i += len;
                    continue;
                }
                if c == b'/'
                    && regex_context(&toks)
                    && let Some(end) = scan_line_regex(b, i)
                {
                    toks.push((Tok::Str(&src[i..end]), line));
                    i = end;
                    continue;
                }
                let op_len = cfamily::punct_len(src, i);
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

/// Regular string with `\(expr)` interpolation (recursively: the
/// interpolated expression may itself contain strings).
fn scan_str(b: &[u8], start: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = start + 1;
    let mut line = line;
    while i < b.len() {
        match b[i] {
            b'\\' if i + 1 < b.len() && b[i + 1] == b'(' => {
                i = scan_interp(b, i + 1, &mut line)?;
            }
            b'\\' => {
                if i + 1 < b.len() && b[i + 1] == b'\n' {
                    line += 1;
                }
                i += 2;
            }
            b'"' => return Ok((i + 1, line)),
            b'\n' => return Err(format!("line {line}: newline in string")),
            _ => i += 1,
        }
    }
    Err(format!("line {line}: unterminated string"))
}

/// Multiline `"""` string: any `\(` interpolation scanned recursively,
/// newlines counted. Stops at the first unescaped `"""`.
fn scan_multiline(b: &[u8], start: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = start + 3;
    let mut line = line;
    while i < b.len() {
        match b[i] {
            b'\\' if i + 1 < b.len() && b[i + 1] == b'(' => {
                i = scan_interp(b, i + 1, &mut line)?;
            }
            b'\\' => {
                if i + 1 < b.len() && b[i + 1] == b'\n' {
                    line += 1;
                }
                i += 2;
            }
            b'"' if i + 2 < b.len() && b[i + 1] == b'"' && b[i + 2] == b'"' => {
                return Ok((i + 3, line));
            }
            b'\n' => {
                line += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    Err(format!("line {line}: unterminated multiline string"))
}

/// Raw string body (opening quote at `start`, `hashes` `#`s expected
/// after the closing quote). `\#(` interpolation is scanned balanced
/// without nested strings (documented limitation).
fn scan_raw(b: &[u8], start: usize, hashes: usize, line: usize) -> Result<(usize, usize), String> {
    let mut i = start + 1;
    let mut line = line;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 + hashes < b.len() && b[i + 1 + hashes] == b'(' {
            let mut hashes_ok = true;
            for k in 1..=hashes {
                if b.get(i + k) != Some(&b'#') {
                    hashes_ok = false;
                }
            }
            if hashes_ok {
                i = scan_interp(b, i + 1 + hashes, &mut line)?;
                continue;
            }
        }
        if b[i] == b'"' {
            let mut ok = true;
            for k in 1..=hashes {
                if b.get(i + k) != Some(&b'#') {
                    ok = false;
                }
            }
            if ok {
                return Ok((i + 1 + hashes, line));
            }
        }
        if b[i] == b'\n' {
            line += 1;
        }
        i += 1;
    }
    Err(format!("line {line}: unterminated raw string"))
}

/// Regex literal `#/.../#` (with any number of leading `#`s). The
/// closing delimiter is `/` followed by the same number of `#`s.
/// `[...]` character classes may contain unescaped `/`. Newlines are
/// counted. Returns (end index, new line).
fn scan_hash_regex(
    b: &[u8],
    start: usize,
    hashes: usize,
    line: usize,
) -> Result<(usize, usize), String> {
    let mut i = start + 1 + hashes;
    let mut line = line;
    let mut in_class = false;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'[' => in_class = true,
            b']' => in_class = false,
            b'/' if !in_class => {
                let mut ok = true;
                for k in 1..=hashes {
                    if b.get(i + k) != Some(&b'#') {
                        ok = false;
                    }
                }
                if ok {
                    return Ok((i + 1 + hashes, line));
                }
            }
            b'\n' => line += 1,
            _ => {}
        }
        i += 1;
    }
    Err(format!("line {line}: unterminated regex literal"))
}

/// Single-line regex `/.../` opened where an expression can start. On
/// failure (no closing `/` before the newline) returns None so the
/// caller can fall back to a division operator.
const fn scan_line_regex(b: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    let mut in_class = false;
    while i < b.len() && b[i] != b'\n' {
        match b[i] {
            b'\\' => i += 1,
            b'[' => in_class = true,
            b']' => in_class = false,
            b'/' if !in_class && i > start + 1 => return Some(i + 1),
            b'/' if !in_class && i == start + 1 => return None,
            _ => {}
        }
        i += 1;
    }
    None
}

/// Balanced `(...)` starting at the opening paren (index `open`),
/// counting newlines. Nested strings inside the interpolation are
/// skipped (recursively). Returns the index past the closing paren.
fn scan_interp(b: &[u8], open: usize, line: &mut usize) -> Result<usize, String> {
    let mut depth = 0usize;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i + 1);
                }
            }
            b'"' => {
                let (end, nl) = scan_str(b, i, *line)?;
                *line = nl;
                i = end;
                continue;
            }
            b'#' => {
                let mut h = 0usize;
                while i + h < b.len() && b[i + h] == b'#' {
                    h += 1;
                }
                if i + h < b.len() && b[i + h] == b'"' {
                    let q = i + h;
                    let (end, nl) = if b[q + 1..].starts_with(b"\"\"\"") {
                        scan_multiline(b, q, *line)?
                    } else {
                        scan_raw(b, q, h, *line)?
                    };
                    *line = nl;
                    i = end;
                    continue;
                }
            }
            b'\n' => *line += 1,
            _ => {}
        }
        i += 1;
    }
    Err(format!("line {}: unterminated interpolation", *line))
}

#[expect(
    clippy::too_many_lines,
    reason = "one pass over the token stream that tracks brace depth, the container stack and the open function span together; a split would share about ten mutable state variables through every helper"
)]
#[expect(
    clippy::many_single_char_names,
    reason = "the cursor, the current byte and token indices keep their conventional one-letter scanner names (b, c, i, j, k)"
)]
fn analyze_swift(src: &str) -> Result<Vec<RawFunction>, String> {
    let (toks, comment_lines) = lex(src)?;
    let mut collector = MetricsCollector::new(&SWIFT_SPEC, 0);
    let mut raws = Vec::new();
    let mut type_stack: Vec<Option<String>> = Vec::new();
    let mut pending_type: Option<String> = None;
    let mut run_start: Option<(usize, usize)> = None;
    let mut i = 0usize;
    while i < toks.len() {
        let (tok, ln) = &toks[i];
        match tok {
            Tok::Newline => {
                i += 1;
            }
            Tok::Kw(k) if *k == "func" || *k == "init" || *k == "deinit" => {
                if *k != "func" && run_start.is_none() && !at_member_boundary(&toks, i) {
                    i += 1;
                    continue;
                }
                let start_line = run_start.map_or(*ln, |(_, l)| l);
                let mut j = i + 1;
                let name = if *k == "func" {
                    let name = match toks.get(j) {
                        Some((Tok::Ident(n) | Tok::Kw(n), _)) => (*n).to_string(),
                        Some((Tok::Op(o), _)) if *o == "." => match toks.get(j + 1) {
                            Some((Tok::Op(o2), _)) => {
                                j += 1;
                                format!(".{o2}")
                            }
                            _ => (*o).to_string(),
                        },
                        Some((Tok::Op(o), _)) => (*o).to_string(),
                        _ => {
                            run_start = None;
                            i += 1;
                            continue;
                        }
                    };
                    j += 1;
                    name
                } else {
                    (*k).to_string()
                };
                j = skip_generics_and_marker(&toks, j);
                let (params, after) = match toks.get(j) {
                    Some((Tok::Op(o), _)) if *o == "(" => {
                        let (n, a) = count_params(&toks, j);
                        (n, a)
                    }
                    _ => (0usize, j),
                };
                let (body_start, end_line, end_idx, has_result, has_body) =
                    find_body(&toks, after, *ln).ok_or_else(|| {
                        format!("line {start_line}: function without body terminator")
                    })?;
                let owner = type_stack.iter().rev().find_map(std::clone::Clone::clone);
                let qualified = match owner {
                    Some(o) => format!("{o}::{name}"),
                    None => name,
                };
                let io_params = params + usize::from(has_result);
                if has_body && body_start < toks.len() {
                    let raw = build_raw(
                        &mut collector,
                        &toks,
                        &comment_lines,
                        body_start,
                        end_idx,
                        start_line,
                        end_line,
                        &qualified,
                        count_u32(io_params),
                    );
                    raws.push(raw);
                }
                run_start = None;
                i = if has_body { body_start - 1 } else { end_idx };
            }
            Tok::Kw(k) if TYPE_KEYWORDS.contains(k) => {
                let name = toks
                    .get(i + 1)
                    .and_then(|t| match t.0 {
                        Tok::Ident(n) => Some(n.to_string()),
                        _ => None,
                    })
                    .unwrap_or_default();
                pending_type = if name.is_empty() { None } else { Some(name) };
                run_start = None;
                i += 1;
            }
            Tok::Kw(k) if MODIFIERS.contains(k) => {
                if run_start.is_none() && at_member_boundary(&toks, i) {
                    run_start = Some((i, *ln));
                }
                i += 1;
            }
            Tok::Op(o) if *o == "@" => {
                if run_start.is_none() && at_member_boundary(&toks, i) {
                    run_start = Some((i, *ln));
                }
                let mut j = i + 1;
                if matches!(toks.get(j), Some((Tok::Ident(_), _))) {
                    j += 1;
                }
                if matches!(toks.get(j), Some((Tok::Op(p), _)) if *p == "(") {
                    let mut depth = 0usize;
                    while j < toks.len() {
                        match &toks[j].0 {
                            Tok::Op(p) if *p == "(" => depth += 1,
                            Tok::Op(p) if *p == ")" => {
                                depth -= 1;
                                if depth == 0 {
                                    j += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                }
                i = j;
            }
            Tok::Op(o) if *o == "{" => {
                type_stack.push(pending_type.take());
                run_start = None;
                i += 1;
            }
            Tok::Op(o) if *o == "}" => {
                type_stack.pop();
                run_start = None;
                i += 1;
            }
            Tok::Op(o) if *o == ";" => {
                run_start = None;
                i += 1;
            }
            _ => {
                let op_modifier = run_start.is_some()
                    && matches!(tok, Tok::Ident(n) if *n == "prefix" || *n == "postfix" || *n == "infix")
                    && matches!(toks.get(i + 1).map(|t| &t.0), Some(Tok::Kw(k)) if *k == "func");
                if !op_modifier {
                    run_start = None;
                }
                i += 1;
            }
        }
    }
    Ok(raws)
}

/// A modifier/attribute run only starts where a member can start:
/// after a newline (Swift's statement separator), `{`, `}`, `;`, or at
/// the top of the file.
fn at_member_boundary(toks: &[(Tok<'_>, usize)], i: usize) -> bool {
    match toks.get(i.wrapping_sub(1)) {
        None | Some((Tok::Newline, _)) => true,
        Some((Tok::Op(o), _)) => *o == "{" || *o == "}" || *o == ";",
        _ => false,
    }
}

/// Skip a generic parameter list `<...>` and a failable marker `?`/`!`
/// after a function name or `init` keyword. Returns the next index.
fn skip_generics_and_marker(toks: &[(Tok<'_>, usize)], mut j: usize) -> usize {
    if matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "<") {
        let mut depth = 0usize;
        while j < toks.len() {
            match &toks[j].0 {
                Tok::Op(o) if *o == "<" => depth += 1,
                Tok::Op(o) if *o == ">" => depth = depth.saturating_sub(1),
                Tok::Op(o) if *o == ">>" => depth = depth.saturating_sub(2),
                Tok::Op(o) if *o == ">>>" => depth = depth.saturating_sub(3),
                _ => {}
            }
            j += 1;
            if depth == 0 {
                break;
            }
        }
    }
    while matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "?" || *o == "!") {
        j += 1;
    }
    j
}

/// Count top-level comma-separated parameter segments. Returns
/// (count, index past the closing paren).
fn count_params(toks: &[(Tok<'_>, usize)], open: usize) -> (usize, usize) {
    let mut depth = 0usize;
    let mut segments = 0usize;
    let mut saw_content = false;
    let mut j = open;
    while j < toks.len() {
        match &toks[j].0 {
            Tok::Op(o) if *o == "(" || *o == "[" || *o == "{" => depth += 1,
            Tok::Op(o) if *o == ")" || *o == "]" || *o == "}" => {
                depth -= 1;
                if depth == 0 {
                    return (segments + usize::from(saw_content), j + 1);
                }
            }
            Tok::Op(o) if *o == "," && depth == 1 => {
                segments += 1;
                saw_content = true;
            }
            Tok::Newline => {}
            _ => saw_content = true,
        }
        j += 1;
    }
    (segments + usize::from(saw_content), j)
}

/// From just past the parameter list, find the body `{...}` or the end
/// of a bodyless (protocol) signature. Returns
/// (`body_start`, `end_line`, `body_end_index`, `has_result`, `has_body`).
fn find_body(
    toks: &[(Tok<'_>, usize)],
    from: usize,
    def_line: usize,
) -> Option<(usize, usize, usize, bool, bool)> {
    let mut depth = 0usize;
    let mut has_result = false;
    let mut j = from;
    let mut last_line = def_line;
    while j < toks.len() {
        let (tok, ln) = &toks[j];
        match tok {
            Tok::Op(o) if *o == "(" || *o == "[" => depth += 1,
            Tok::Op(o) if *o == ")" || *o == "]" => depth -= 1,
            Tok::Op(o) if *o == "->" && depth == 0 => has_result = true,
            Tok::Op(o) if *o == "{" && depth == 0 => {
                let mut bd = 0usize;
                let mut k = j;
                while k < toks.len() {
                    match &toks[k].0 {
                        Tok::Op(o) if *o == "{" => bd += 1,
                        Tok::Op(o) if *o == "}" => {
                            bd -= 1;
                            if bd == 0 {
                                return Some((j + 1, toks[k].1, k, has_result, true));
                            }
                        }
                        _ => {}
                    }
                    k += 1;
                }
                return None;
            }
            Tok::Newline if depth == 0 => {
                let type_continues = matches!(
                    toks[..j].iter().rev().find(|t| !matches!(t.0, Tok::Newline)).map(|t| &t.0),
                    Some(Tok::Op(o)) if *o == "->" || *o == "<" || *o == ","
                );
                if j > from && !type_continues && !signature_continues(toks, j) {
                    return Some((j, last_line, j, has_result, false));
                }
            }
            Tok::Op(o) if *o == "}" && depth == 0 => {
                return Some((j, last_line, j, has_result, false));
            }
            Tok::Op(o) if *o == ";" && depth == 0 => {
                return Some((j, last_line, j, has_result, false));
            }
            Tok::Kw(k)
                if depth == 0
                    && (TYPE_KEYWORDS.contains(k)
                        || matches!(
                            &**k,
                            "func"
                                | "init"
                                | "deinit"
                                | "var"
                                | "let"
                                | "import"
                                | "subscript"
                                | "typealias"
                                | "associatedtype"
                        )) =>
            {
                return Some((j, last_line, j, has_result, false));
            }
            _ => {}
        }
        if !matches!(tok, Tok::Newline) {
            last_line = *ln;
        }
        j += 1;
    }
    Some((
        toks.len(),
        last_line,
        toks.len().saturating_sub(1),
        has_result,
        false,
    ))
}

/// After a depth-0 newline in a signature, Swift may continue the
/// signature (`->` type, effects, `where` clause) or open the body brace
/// on a later line. Only then does the scan keep going; otherwise the
/// member is bodyless (protocol) and ends here.
fn signature_continues(toks: &[(Tok<'_>, usize)], at_newline: usize) -> bool {
    let mut j = at_newline + 1;
    while matches!(toks.get(j), Some((Tok::Newline, _))) {
        j += 1;
    }
    match toks.get(j).map(|t| &t.0) {
        Some(Tok::Op(o)) => *o == "->" || *o == "{" || *o == ">",
        Some(Tok::Kw(k)) => matches!(*k, "where" | "throws" | "rethrows" | "async"),
        Some(Tok::Ident(n)) => matches!(*n, "some" | "any"),
        _ => false,
    }
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
                    let is_call = match callee {
                        Some(Tok::Ident(_)) => true,
                        Some(Tok::Op(p)) => *p == ")" || *p == "]",
                        _ => false,
                    };
                    if is_call {
                        let is_method = matches!(
                            toks.get(idx.wrapping_sub(2)).map(|t| &t.0),
                            Some(Tok::Op(p)) if *p == "." || *p == "?."
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
        language: halmi_core::report::Language::Swift,
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

const SWIFT_SPEC: halmi_lex::LangSpec = halmi_lex::LangSpec {
    decl_op: "func",
    keyword_operators: &KEYWORD_OPERATORS,
    cc_keywords: &["if", "guard", "for", "while", "case", "where"],
    separator_ops: halmi_lex::PAREN_SEPARATORS,
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
