//! halmi-ruby: Ruby language adapter.
//!
//! Hand lexer + declaration scanner on [`halmi_lex`] shared machinery.
//! `def` triggers candidates; bodies are `end`-terminated blocks
//! (def/do/if/unless/case/while/until/for/begin/class/module and `{`
//! open; `end` closes; modifier if/unless/while/until/rescue do not
//! open). Zero dependencies; no tree-sitter (the tree-sitter-ruby
//! oracle is a test-side differential only).
//!
//! * Definitions: `method` / `singleton_method` parity: instance
//!   methods, `def self.x` singleton methods, setters (`name=`),
//!   operators, `class << self` members. Lambdas/blocks excluded.
//! * Qualification: module/class container chain (`Demo::Widget::area`).
//! * `decl_op`: `def`.

#![deny(unsafe_code)]

use halmi_core::adapter::{LanguageAdapter, SourceAnalysis};
use halmi_core::report::{Language, RawFunction, count_f64, count_u32};
use halmi_lex::{LangSpec, LocMode, MetricsCollector};

#[derive(Debug, Clone, Copy, Default)]
pub struct RubyAdapter;

impl LanguageAdapter for RubyAdapter {
    fn language(&self) -> &'static str {
        "Ruby"
    }

    fn analyze(&self, path: &str, src: &str) -> SourceAnalysis {
        match analyze_ruby(src) {
            Ok(mut raws) => {
                for raw in &mut raws {
                    raw.path = path.to_string();
                    raw.language = Language::Ruby;
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

const RUBY_SPEC: LangSpec = LangSpec {
    decl_op: "def",
    keyword_operators: CONTROL_KEYWORDS,
    cc_keywords: &[
        "if", "elsif", "unless", "when", "while", "until", "for", "rescue",
    ],
    separator_ops: &["(", ")", "[", "]", "{", "}", ",", ";"],
    cc_ops: &["&&", "||"],
    loc_mode: LocMode::Physical,
};

const CONTROL_KEYWORDS: &[&str] = &[
    "if", "elsif", "else", "unless", "case", "when", "while", "until", "for", "in", "do", "begin",
    "rescue", "ensure", "return", "break", "next", "redo", "retry", "yield", "raise", "throw",
    "catch", "and", "or", "not", "super", "self", "new", "defined?", "lambda", "proc",
];

const ALL_KEYWORDS: &[&str] = &[
    "BEGIN", "END", "__FILE__", "__LINE__", "alias", "and", "begin", "break", "case", "class",
    "def", "defined?", "do", "else", "elsif", "end", "ensure", "false", "for", "if", "in",
    "module", "next", "nil", "not", "or", "redo", "rescue", "retry", "return", "self", "super",
    "then", "true", "undef", "unless", "until", "when", "while", "yield",
];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok<'a> {
    Ident(&'a str),
    Sym(&'a str),
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
            b'\\' if i + 1 < b.len() && b[i + 1] == b'\n' => {
                line += 1;
                i += 2;
            }
            b'\\' if i + 2 < b.len() && b[i + 1] == b'\r' && b[i + 2] == b'\n' => {
                line += 1;
                i += 3;
            }
            b'#' => {
                if line >= 1 && line <= n_lines {
                    comment_lines[line - 1] = true;
                }
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'_' if src[i..].starts_with("__END__")
                && matches!(b.get(i + 7), None | Some(&b'\n')) =>
            {
                break;
            }
            b'`' => {
                let (end, nl) = lex_dq(b, i, line, b'`')?;
                line = nl;
                toks.push((Tok::Str(&src[i..end]), line));
                i = end;
            }
            b'"' => {
                let (end, nl) = lex_dq(b, i, line, b'"')?;
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
            b'%' => {
                if let Some((end, nl)) = lex_percent_literal(b, i, line)? {
                    line = nl;
                    toks.push((Tok::Str(&src[i..end]), line));
                    i = end;
                } else {
                    let op_len = halmi_lex::cfamily::punct_len(src, i);
                    toks.push((Tok::Op(&src[i..i + op_len]), line));
                    i += op_len;
                }
            }
            b'<' if heredoc_ahead(src, i) => {
                let (end, nl) = lex_heredoc(b, i, line)?;
                line = nl;
                toks.push((Tok::Str(&src[i..end]), line));
                i = end;
            }
            b':' if i + 1 < b.len()
                && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_')
                && i > 0
                && !(b[i - 1].is_ascii_alphanumeric()
                    || b[i - 1] == b'_'
                    || b[i - 1] == b')'
                    || b[i - 1] == b']') =>
            {
                let (tok, end) = lex_plain_symbol(src, i);
                toks.push((tok, line));
                i = end;
            }
            b':' if i + 1 < b.len() && b[i + 1] == b'"' => {
                let (end, nl) = lex_dq(b, i + 1, line, b'"')?;
                line = nl;
                toks.push((Tok::Sym(&src[i + 2..end - 1]), line));
                i = end;
            }
            b'$' if i + 1 < b.len() && !b[i + 1].is_ascii_whitespace() => {
                let (tok, end) = lex_global(src, i);
                toks.push((tok, line));
                i = end;
            }
            b'@' if i + 1 < b.len()
                && (b[i + 1] == b'@' || b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_') =>
            {
                let (tok, end) = lex_ivar(src, i);
                toks.push((tok, line));
                i = end;
            }
            b'?' if i + 1 < b.len()
                && b[i + 1] != b' '
                && b[i + 1] != b'\t'
                && b[i + 1] != b'\n'
                && match toks.last().map(|t| &t.0) {
                    Some(Tok::Kw(_) | Tok::Newline) | None => true,
                    Some(Tok::Op(o)) => !matches!(&**o, ")" | "]" | "}"),
                    _ => false,
                } =>
            {
                let (tok, end) = lex_char(src, i, line)?;
                toks.push((tok, line));
                i = end;
            }
            c if c.is_ascii_digit() => {
                let end = lex_number(b, i);
                toks.push((Tok::Number(&src[i..end]), line));
                i = end;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let (tok, end) = lex_word(src, i, after_def_keyword(&toks));
                toks.push((tok, line));
                i = end;
            }
            b'/' if regex_context(&toks) => {
                if let Some((end, newlines)) = lex_regex(b, i) {
                    toks.push((Tok::Str(&src[i..end]), line));
                    line += newlines;
                    i = end;
                } else {
                    let op_len = halmi_lex::cfamily::punct_len(src, i);
                    toks.push((Tok::Op(&src[i..i + op_len]), line));
                    i += op_len;
                }
            }
            _ => {
                if !c.is_ascii() {
                    let len = halmi_lex::cfamily::utf8_len(c);
                    toks.push((Tok::Str(&src[i..i + len]), line));
                    i += len;
                    continue;
                }
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

fn lex_percent_literal(
    b: &[u8],
    start: usize,
    line: usize,
) -> Result<Option<(usize, usize)>, String> {
    let Some(&next) = b.get(start + 1) else {
        return Ok(None);
    };
    let has_letter = matches!(
        next,
        b'w' | b'W' | b'i' | b'I' | b'q' | b'Q' | b'r' | b'R' | b's' | b'S' | b'x'
    );
    let open = if has_letter {
        let Some(&open) = b.get(start + 2) else {
            return Ok(None);
        };
        if !matches!(open, b'(' | b'[' | b'{' | b'<' | b'!' | b'/' | b'|') {
            return Ok(None);
        }
        open
    } else {
        if !matches!(next, b'(' | b'[' | b'{' | b'<' | b'!' | b'/' | b'|') {
            return Ok(None);
        }
        next
    };
    let mut j = if has_letter { start + 3 } else { start + 2 };
    let close = match open {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        b'<' => b'>',
        b'!' => b'!',
        o => o,
    };
    let mut d = 1usize;
    let mut nl = line;
    while j < b.len() && d > 0 {
        if b[j] == b'\\' {
            j += 2;
            continue;
        }
        if b[j] == close {
            d -= 1;
        } else if b[j] == open {
            d += 1;
        } else if b[j] == b'\n' {
            nl += 1;
        }
        j += 1;
    }
    if d != 0 {
        return Err(format!("line {line}: unterminated %-literal"));
    }
    Ok(Some((j, nl)))
}

const fn lex_regex(b: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut j = start + 1;
    let mut in_class = false;
    let mut newlines = 0usize;
    while j < b.len() {
        if b[j] == b'\n' {
            newlines += 1;
            if newlines > 60 {
                break;
            }
            j += 1;
            continue;
        }
        match b[j] {
            b'\\' => j += 2,
            b'[' => {
                in_class = true;
                j += 1;
            }
            b']' => {
                in_class = false;
                j += 1;
            }
            b'/' if !in_class && j > start + 1 => {
                let after = j + 1;
                let mut f = after;
                while f < b.len()
                    && matches!(b[f], b'x' | b'i' | b'm' | b'o' | b'u' | b'e' | b's' | b'n')
                {
                    f += 1;
                }
                let single_line = newlines == 0;
                let ends_clean = f >= b.len()
                    || b[f] == b'\n'
                    || b[f] == b' '
                    || b[f] == b','
                    || b[f] == b')'
                    || b[f] == b'.';
                if single_line || ends_clean {
                    return Some((f, newlines));
                }
                j += 1;
            }
            _ => j += 1,
        }
    }
    None
}

fn lex_plain_symbol(src: &str, start: usize) -> (Tok<'_>, usize) {
    let b = src.as_bytes();
    let mut end = start + 1;
    while end < b.len() && (b[end].is_ascii_alphanumeric() || b[end] == b'_') {
        end += 1;
    }
    if end < b.len()
        && (b[end] == b'?' || b[end] == b'!' || b[end] == b'=')
        && b.get(end + 1) != Some(&b'=')
    {
        end += 1;
    }
    (Tok::Sym(&src[start + 1..end]), end)
}

fn lex_global(src: &str, start: usize) -> (Tok<'_>, usize) {
    let b = src.as_bytes();
    let mut end = start + 1;
    if b[end].is_ascii_alphanumeric() || b[end] == b'_' {
        while end < b.len() && (b[end].is_ascii_alphanumeric() || b[end] == b'_') {
            end += 1;
        }
    } else {
        end += 1;
    }
    (Tok::Ident(&src[start..end]), end)
}

fn lex_ivar(src: &str, start: usize) -> (Tok<'_>, usize) {
    let b = src.as_bytes();
    let mut end = start + 1;
    if b[end] == b'@' {
        end += 1;
    }
    while end < b.len() && (b[end].is_ascii_alphanumeric() || b[end] == b'_') {
        end += 1;
    }
    (Tok::Ident(&src[start..end]), end)
}

fn lex_char(src: &str, start: usize, line: usize) -> Result<(Tok<'_>, usize), String> {
    let b = src.as_bytes();
    let mut end = start + 1;
    if end < b.len() && b[end] == b'\\' {
        end += 1;
    }
    if end >= b.len() {
        return Err(format!("line {line}: unterminated character literal"));
    }
    let char_len = halmi_lex::cfamily::utf8_len(b[end]);
    if end + char_len > b.len() {
        return Err(format!("line {line}: unterminated character literal"));
    }
    end += char_len;
    Ok((Tok::Str(&src[start..end]), end))
}
/// Whether the next word would be the method name of a `def` keyword,
/// possibly behind a receiver chain (`def self.x`, `def Const.x`).
fn after_def_keyword(toks: &[(Tok<'_>, usize)]) -> bool {
    let mut k = toks.len();
    while k > 0 {
        k -= 1;
        if matches!(&toks[k].0, Tok::Kw("def")) {
            return true;
        }
        let receiver = matches!(toks.get(k + 1).map(|t| &t.0), Some(Tok::Op("." | "::")));
        if !receiver {
            return false;
        }
    }
    false
}

/// `def_name`: when `fold_setter` holds, an adjacent `=` (not `==`, `=>`,
/// `=~`) is part of the name (`def name= o`, `def name=(v)`); `def name = o`
/// with intervening whitespace stays an endless definition. This mirrors the
/// Ruby parser, which only treats `name=` as a setter in a `def` head.
fn lex_word(src: &str, start: usize, fold_setter: bool) -> (Tok<'_>, usize) {
    let b = src.as_bytes();
    let mut end = start;
    while end < b.len() && (b[end].is_ascii_alphanumeric() || b[end] == b'_') {
        end += 1;
    }
    if end < b.len()
        && (b[end] == b'?' || b[end] == b'!')
        && (end + 1 >= b.len() || b[end + 1] != b'=')
    {
        end += 1;
    }
    if fold_setter
        && b.get(end) == Some(&b'=')
        && !matches!(b.get(end + 1), Some(b'=' | b'>' | b'~'))
    {
        end += 1;
    }
    let word = &src[start..end];
    let tok = if ALL_KEYWORDS.binary_search(&word).is_ok() {
        Tok::Kw(word)
    } else {
        Tok::Ident(word)
    };
    (tok, end)
}

/// A `/` opens a regex literal at expression position: after an
/// operator, opening punctuator, keyword, or at a line/statement start.
/// After an identifier, literal, or closer it is division.
fn regex_context(toks: &[(Tok<'_>, usize)]) -> bool {
    if matches!(toks.last().map(|t| &t.0), Some(Tok::Newline)) {
        return true;
    }
    let mut j = toks.len();
    while j > 0 {
        j -= 1;
        match &toks[j].0 {
            Tok::Newline => {}
            Tok::Ident(_) | Tok::Sym(_) | Tok::Number(_) | Tok::Str(_) => return false,
            Tok::Op(o) => {
                return !matches!(&**o, ")" | "]" | "}");
            }
            Tok::Kw(_) => return true,
        }
    }
    true
}

fn heredoc_ahead(src: &str, i: usize) -> bool {
    let rest = &src[i..];
    if !rest.starts_with("<<") {
        return false;
    }
    let after = &rest[2..];
    let after = after
        .strip_prefix('-')
        .or_else(|| after.strip_prefix('~'))
        .unwrap_or(after);
    if after.is_empty() {
        return false;
    }
    let first = after.as_bytes()[0];
    first.is_ascii_alphabetic() || first == b'\'' || first == b'"'
}

/// `close` is the delimiter byte that ends the literal: `"` for double-quoted
/// strings and `:"sym"`, `` ` `` for command literals. Interpolation bodies
/// may contain `"` strings whatever the outer delimiter.
fn lex_dq(b: &[u8], q: usize, line: usize, close: u8) -> Result<(usize, usize), String> {
    let mut i = q + 1;
    let mut line = line;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'#' if i + 1 < b.len()
                && (b[i + 1] == b'{' || b[i + 1] == b'$' || b[i + 1] == b'@') =>
            {
                if b[i + 1] == b'{' {
                    let mut d = 0usize;
                    while i < b.len() {
                        if b[i] == b'{' {
                            d += 1;
                        } else if b[i] == b'}' {
                            d -= 1;
                            if d == 0 {
                                break;
                            }
                        } else if b[i] == b'"' {
                            i += 1;
                            while i < b.len() && b[i] != b'"' {
                                if b[i] == b'\\' {
                                    i += 1;
                                }
                                i += 1;
                            }
                        }
                        i += 1;
                    }
                    i += 1;
                } else {
                    i += 2;
                    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                        i += 1;
                    }
                }
            }
            c if c == close => return Ok((i + 1, line)),
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
    let mut i = start + 2;
    let flex = b[i] == b'~' || b[i] == b'-';
    let indentable = flex;
    if flex {
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
    let id = b[id_start..i].to_vec();
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
        if indentable {
            while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                j += 1;
            }
        }
        if b[j..].starts_with(&id[..])
            && (j + id.len() >= b.len() || b[j + id.len()] == b'\n' || b[j + id.len()] == b'\r')
        {
            return Ok((j + id.len(), line));
        }
        while i < b.len() && b[i] != b'\n' {
            i += 1;
        }
    }
    Err(format!("line {line}: unterminated heredoc"))
}

fn lex_number(b: &[u8], start: usize) -> usize {
    let mut i = start;
    if b[i] == b'0'
        && i + 1 < b.len()
        && matches!(
            b[i + 1],
            b'x' | b'X' | b'b' | b'B' | b'o' | b'O' | b'd' | b'D'
        )
    {
        let radix_char = b[i + 1];
        i += 2;
        let ok = |c: u8| match radix_char {
            b'x' | b'X' => c.is_ascii_hexdigit(),
            b'b' | b'B' => c == b'0' || c == b'1',
            b'o' | b'O' => (b'0'..=b'7').contains(&c),
            _ => c.is_ascii_digit(),
        };
        while i < b.len() && (ok(b[i]) || b[i] == b'_') {
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

/// Whether a conditional keyword at `j` opens a block (as opposed to
/// a trailing modifier): openers appear at statement start.
fn is_opener(toks: &[(Tok<'_>, usize)], j: usize) -> bool {
    let Some(prev) = toks.get(j.wrapping_sub(1)) else {
        return true;
    };
    match &prev.0 {
        Tok::Newline => true,
        Tok::Op(o) => matches!(&**o, ";" | "{" | "(" | "[" | "=" | "&&" | "||" | "," | "=>"),
        Tok::Kw(k) => matches!(
            &**k,
            "then"
                | "do"
                | "else"
                | "elsif"
                | "begin"
                | "and"
                | "or"
                | "not"
                | "in"
                | "when"
                | "rescue"
        ),
        _ => false,
    }
}

/// Block-end search starting at `start` with `open` already on the
/// stack (a def's parameter list may contain `self.class`-style
/// keywords that must not count as openers).
fn find_end_from(toks: &[(Tok<'_>, usize)], open: usize, start: usize) -> Option<usize> {
    let mut stack: Vec<usize> = vec![open];
    let mut k = start;
    while k < toks.len() {
        match &toks[k].0 {
            Tok::Kw(kw) => match &**kw {
                "begin" | "case" => {
                    stack.push(k);
                }
                "def" => match def_header(toks, k)
                    .and_then(|(_, _, j)| endless_body_end_after(toks, j))
                {
                    Some(last) => k = last,
                    None => stack.push(k),
                },
                "do" => {
                    let ln = toks[k].1;
                    let loop_do = stack.last().is_some_and(|open| {
                        matches!(&toks[*open].0, Tok::Kw(open_kw) if matches!(&**open_kw, "while" | "until" | "for"))
                            && toks[*open].1 == ln
                    });
                    if !loop_do {
                        stack.push(k);
                    }
                }
                "class" | "module" | "if" | "unless" | "while" | "until" | "for" => {
                    if is_opener(toks, k) {
                        stack.push(k);
                    }
                }
                "end" => {
                    stack.pop();
                    if stack.is_empty() {
                        return Some(k);
                    }
                }
                _ => {}
            },
            Tok::Op(o) if *o == "{" => stack.push(k),
            Tok::Op(o) if *o == "}" => {
                stack.pop();
                if stack.is_empty() {
                    return Some(k);
                }
            }
            _ => {}
        }
        k += 1;
    }
    None
}

/// Method name, distinct parameter names and the index just past the header
/// of the `def` at `def_i`: the receiver chain (`self.`, `opts.`, `Const::`),
/// the setter mark (`name=(`) and the parenthesised parameter list. `def x =`
/// and paren-less parameter lists leave `j` at the `=` / first parameter.
fn def_header(toks: &[(Tok<'_>, usize)], def_i: usize) -> Option<(String, Vec<String>, usize)> {
    let mut j = def_i + 1;
    let mut name = match toks.get(j).map(|t| &t.0) {
        Some(Tok::Ident(n) | Tok::Kw(n) | Tok::Sym(n)) => (*n).to_string(),
        Some(Tok::Op(o)) => (*o).to_string(),
        _ => return None,
    };
    j += 1;
    while let Some((Tok::Op(o), _)) = toks.get(j)
        && matches!(&**o, "." | "::")
        && let Some(next) = toks.get(j + 1).map(|t| &t.0)
        && (matches!(next, Tok::Ident(_) | Tok::Kw(_))
            || (&**o == "." && matches!(next, Tok::Op(_))))
    {
        name = match next {
            Tok::Ident(n) | Tok::Kw(n) => (*n).to_string(),
            Tok::Op(o) => (*o).to_string(),
            _ => unreachable!("guarded above"),
        };
        j += 2;
    }
    if matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "=")
        && matches!(toks.get(j + 1), Some((Tok::Op(p), _)) if *p == "(")
    {
        name.push('=');
        j += 1;
    }
    let mut params: Vec<String> = Vec::new();
    if matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "(") {
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
    }
    Some((name, params, j))
}

/// For a header ending at `j`: when `toks[j]` is `=`, the definition is endless
/// (`def name(params) = expr`); returns the index of the last body token.
fn endless_body_end_after(toks: &[(Tok<'_>, usize)], j: usize) -> Option<usize> {
    if !matches!(toks.get(j), Some((Tok::Op(o), _)) if *o == "=") {
        return None;
    }
    endless_body_end(toks, j + 1)
}

/// End of an endless definition's body (`def name(params) = expr`, Ruby 3):
/// the index of the last token of the logical line that starts at `start`.
/// A newline ends the body only at bracket depth zero, when the token before
/// it can end an expression and the next line does not continue it with a
/// leading `.`; `;` ends the body anywhere. Heredoc bodies are single tokens,
/// so they are included.
fn endless_body_end(toks: &[(Tok<'_>, usize)], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut k = start;
    let mut last: Option<usize> = None;
    while k < toks.len() {
        match &toks[k].0 {
            Tok::Op(o) if matches!(&**o, "(" | "[" | "{") => depth += 1,
            Tok::Op(o) if matches!(&**o, ")" | "]" | "}") => depth = depth.saturating_sub(1),
            Tok::Op(o) if *o == ";" && depth == 0 => break,
            Tok::Newline if depth == 0 => {
                let leads_with_dot = matches!(
                    toks.get(k + 1).map(|t| &t.0),
                    Some(Tok::Op(o)) if matches!(&**o, "." | "&.")
                );
                let open_ended = match last.map(|l| &toks[l].0) {
                    Some(Tok::Op(o)) => !matches!(&**o, ")" | "]" | "}"),
                    Some(Tok::Kw(kw)) => matches!(
                        &**kw,
                        "if" | "unless"
                            | "while"
                            | "until"
                            | "and"
                            | "or"
                            | "not"
                            | "then"
                            | "else"
                            | "elsif"
                            | "rescue"
                            | "do"
                    ),
                    _ => false,
                };
                if !(leads_with_dot || open_ended) {
                    break;
                }
            }
            _ => {}
        }
        if !matches!(toks[k].0, Tok::Newline) {
            last = Some(k);
        }
        k += 1;
    }
    last
}

#[expect(
    clippy::too_many_lines,
    reason = "one pass over the token stream that tracks brace depth, the container stack and the open function span together; a split would share about ten mutable state variables through every helper"
)]
fn analyze_ruby(src: &str) -> Result<Vec<RawFunction>, String> {
    #[derive(Clone)]
    enum Blk {
        Container(Option<String>),
        Plain,
    }
    let (toks, comment_lines) = lex(src)?;
    let mut collector = MetricsCollector::new(&RUBY_SPEC, 0);
    let mut raws = Vec::new();
    let mut stack: Vec<Blk> = Vec::new();
    let mut i = 0usize;
    while i < toks.len() {
        match &toks[i].0 {
            Tok::Kw(k) if *k == "def" => {
                let ln = toks[i].1;
                let Some((name, params, j)) = def_header(&toks, i) else {
                    stack.push(Blk::Plain);
                    i += 1;
                    continue;
                };
                let owner = stack
                    .iter()
                    .filter_map(|b| match b {
                        Blk::Container(Some(n)) => Some(n.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("::");
                let qualified = if owner.is_empty() {
                    name
                } else {
                    format!("{owner}::{name}")
                };
                let (end_idx, end_line, endless) =
                    if let Some(last) = endless_body_end_after(&toks, j) {
                        (last + 1, toks[last].1, true)
                    } else {
                        let Some(end_idx) = find_end_from(&toks, i, j) else {
                            return Err(format!("line {ln}: unterminated def `{qualified}`"));
                        };
                        (end_idx, toks[end_idx].1, false)
                    };
                let decl_line = toks[i].1;
                let io_params = count_u32(params.len() + 1);
                let raw = build_raw(
                    &mut collector,
                    &toks,
                    &comment_lines,
                    i + 1,
                    end_idx,
                    decl_line,
                    end_line,
                    &qualified,
                    io_params,
                );
                raws.push(raw);
                if endless {
                    i = end_idx;
                } else {
                    stack.push(Blk::Plain);
                    i += 1;
                }
            }
            Tok::Kw(k) if *k == "class" || *k == "module" => {
                if !is_opener(&toks, i) {
                    i += 1;
                    continue;
                }
                let name = match toks.get(i + 1).map(|t| &t.0) {
                    Some(Tok::Ident(n)) => Some((*n).to_string()),
                    _ => None,
                };
                stack.push(Blk::Container(name));
                i += 1;
            }
            Tok::Kw(k)
                if matches!(&**k, "if" | "unless" | "while" | "until" | "for")
                    && is_opener(&toks, i) =>
            {
                stack.push(Blk::Plain);
                i += 1;
            }
            Tok::Kw(k) if matches!(&**k, "do" | "begin" | "case") => {
                stack.push(Blk::Plain);
                i += 1;
            }
            Tok::Kw(k) if *k == "end" => {
                stack.pop();
                i += 1;
            }
            Tok::Op(o) if *o == "{" => {
                stack.push(Blk::Plain);
                i += 1;
            }
            Tok::Op(o) if *o == "}" => {
                stack.pop();
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
            Tok::Ident(n) | Tok::Sym(n) => collector.token(&halmi_lex::Tok::Ident(n), *ln),
            Tok::Number(n) | Tok::Str(n) => {
                collector.token(&halmi_lex::Tok::Str(n), *ln);
            }
            Tok::Kw(k) => collector.token(&halmi_lex::Tok::Keyword(k), *ln),
            Tok::Op(o) => match *o {
                "{" => collector.open_block(*ln),
                "}" => collector.close_block(*ln),
                "(" => {
                    let callee = toks.get(idx.wrapping_sub(1)).map(|t| &t.0);
                    let is_method = matches!(toks.get(idx.wrapping_sub(2)), Some((Tok::Op(p), _)) if *p == "." || *p == "&.");
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
        language: Language::Ruby,
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
