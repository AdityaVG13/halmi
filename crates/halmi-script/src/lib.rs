//! halmi-script: the JavaScript/TypeScript adapter for halmi.
//!
//! One lexer, one structure pass, one classifier. A [`Dialect`] selected
//! from the file extension controls the keyword table, JSX enablement, and
//! the reported language; everything else is shared code.
//!
//! Contract: the shared [`LanguageAdapter`] port (pure, never panics,
//! degraded files become error entries). Classification is token-faithful;
//! see CONTRACT.md for the rules and no-claims.

#![deny(unsafe_code)]

use halmi_core::adapter::{LanguageAdapter, SourceAnalysis};
use halmi_core::report::{Language, RawFunction, count_f64, count_u32};
use halmi_lex::MetricsCollector;

/// The JavaScript/TypeScript adapter.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScriptAdapter;

/// Lexical dialect, selected from the file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Js,
    Jsx,
    Ts,
    Tsx,
}

impl Dialect {
    #[must_use]
    pub fn from_path(path: &str) -> Self {
        let ext = path.rsplit('.').next().unwrap_or("");
        match ext {
            "tsx" => Self::Tsx,
            "ts" | "mts" | "cts" => Self::Ts,
            "jsx" => Self::Jsx,
            _ => Self::Js,
        }
    }

    const fn has_jsx(self) -> bool {
        matches!(self, Self::Jsx | Self::Tsx)
    }

    const fn is_ts(self) -> bool {
        matches!(self, Self::Ts | Self::Tsx)
    }

    const fn language(self) -> Language {
        if self.is_ts() {
            Language::TypeScript
        } else {
            Language::JavaScript
        }
    }
}

impl LanguageAdapter for ScriptAdapter {
    fn language(&self) -> &'static str {
        "JavaScript/TypeScript"
    }

    fn analyze(&self, path: &str, src: &str) -> SourceAnalysis {
        let dialect = Dialect::from_path(path);
        match analyze_script(src, dialect) {
            Ok(mut raws) => {
                for raw in &mut raws {
                    raw.path = path.to_string();
                    raw.language = dialect.language();
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
    TemplateChunk(&'a str),
    Regex(&'a str),
    Punct(&'a str),
    Keyword(&'a str),
    JsxText(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BraceKind {
    Block,
    Object,
    Class,
}

const JS_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "async",
    "await",
    "of",
    "static",
    "get",
    "set",
    "true",
    "false",
    "null",
];

const TS_KEYWORDS: &[&str] = &[
    "interface",
    "type",
    "enum",
    "namespace",
    "module",
    "declare",
    "abstract",
    "implements",
    "readonly",
    "public",
    "private",
    "protected",
    "as",
    "satisfies",
    "keyof",
    "infer",
    "is",
    "asserts",
    "unique",
    "override",
    "accessor",
    "out",
];

const VALUE_KEYWORDS: &[&str] = &["this", "super", "true", "false", "null"];

const PUNCTUATORS: &[&str] = &[
    ">>>=", "...", "===", "!==", "**=", "&&=", "||=", "??=", ">>>", "<<=", ">>=", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
    "**", "<<", ">>", "(", ")", "{", "}", "[", "]", ";", ",", "<", ">", "+", "-", "*", "/", "%",
    "&", "|", "^", "!", "~", "?", ":", "=", ".", "@", "#",
];

const fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

const fn is_ident_start(c: u8) -> bool {
    c == b'_' || c == b'$' || c.is_ascii_alphabetic() || c >= 0x80
}

const fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c == b'$' || c.is_ascii_alphanumeric() || c >= 0x80
}

struct Lexer<'a> {
    src: &'a str,
    b: &'a [u8],
    i: usize,
    line: usize,
    dialect: Dialect,
    toks: Vec<(Tok<'a>, usize)>,
    comment_lines: Vec<bool>,
    expr_pos: bool,
    brace_stack: Vec<BraceKind>,
    brace_kinds: Vec<BraceKind>,
    pending_class: bool,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str, dialect: Dialect) -> Self {
        Lexer {
            src,
            b: src.as_bytes(),
            i: 0,
            line: 1,
            dialect,
            toks: Vec::new(),
            comment_lines: vec![false; src.lines().count().max(1)],
            expr_pos: true,
            brace_stack: Vec::new(),
            brace_kinds: Vec::new(),
            pending_class: false,
        }
    }

    fn mark_comment(&mut self) {
        if self.line >= 1 && self.line <= self.comment_lines.len() {
            self.comment_lines[self.line - 1] = true;
        }
    }

    fn skip_ws_and_comments(&mut self) -> Result<(), String> {
        loop {
            if self.i >= self.b.len() {
                return Ok(());
            }
            match self.b[self.i] {
                b' ' | b'\t' | b'\r' => self.i += 1,
                b'\n' => {
                    self.line += 1;
                    self.i += 1;
                }
                b'/' if self.b.get(self.i + 1) == Some(&b'/') => {
                    self.mark_comment();
                    while self.i < self.b.len() && self.b[self.i] != b'\n' {
                        self.i += 1;
                    }
                }
                b'/' if self.b.get(self.i + 1) == Some(&b'*') => {
                    self.mark_comment();
                    self.i += 2;
                    while self.i + 1 < self.b.len()
                        && !(self.b[self.i] == b'*' && self.b[self.i + 1] == b'/')
                    {
                        if self.b[self.i] == b'\n' {
                            self.line += 1;
                            self.mark_comment();
                        }
                        self.i += 1;
                    }
                    if self.i + 1 >= self.b.len() {
                        return Err(format!("line {}: unterminated block comment", self.line));
                    }
                    self.i += 2;
                }
                _ => return Ok(()),
            }
        }
    }

    fn run(&mut self) -> Result<(), String> {
        while self.i < self.b.len() {
            self.skip_ws_and_comments()?;
            if self.i >= self.b.len() {
                break;
            }
            self.next_token()?;
        }
        Ok(())
    }

    fn push_tok(&mut self, tok: Tok<'a>) {
        self.toks.push((tok, self.line));
    }

    fn next_token(&mut self) -> Result<(), String> {
        let c = self.b[self.i];
        match c {
            b'\'' | b'"' => self.lex_string(c),
            b'`' => self.lex_template(),
            b'0'..=b'9' => {
                self.lex_number();
                Ok(())
            }
            b'.' if self.b.get(self.i + 1).is_some_and(u8::is_ascii_digit) => {
                self.lex_number();
                Ok(())
            }
            _ if is_ident_start(c) => {
                self.lex_ident();
                Ok(())
            }
            b'/' => {
                if self.expr_pos {
                    self.lex_regex()
                } else {
                    self.lex_punct()
                }
            }
            b'<' if self.dialect.has_jsx()
                && self.expr_pos
                && self
                    .b
                    .get(self.i + 1)
                    .is_some_and(|n| is_ident_start(*n) || *n == b'>') =>
            {
                self.lex_jsx()
            }
            _ => self.lex_punct(),
        }
    }

    fn lex_string(&mut self, quote: u8) -> Result<(), String> {
        let start = self.i;
        self.i += 1;
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'\\' => self.i += 2,
                b'\n' => return Err(format!("line {}: newline in string", self.line)),
                c if c == quote => {
                    self.i += 1;
                    self.push_tok(Tok::Str(&self.src[start..self.i]));
                    self.expr_pos = false;
                    return Ok(());
                }
                _ => self.i += 1,
            }
        }
        Err(format!("line {}: unterminated string", self.line))
    }

    fn lex_template(&mut self) -> Result<(), String> {
        self.i += 1;
        let mut chunk_start = self.i;
        loop {
            if self.i >= self.b.len() {
                return Err(format!("line {}: unterminated template", self.line));
            }
            match self.b[self.i] {
                b'\\' => self.i += 2,
                b'`' => {
                    if self.i > chunk_start {
                        self.push_tok(Tok::TemplateChunk(&self.src[chunk_start..self.i]));
                    }
                    self.i += 1;
                    self.expr_pos = false;
                    return Ok(());
                }
                b'$' if self.b.get(self.i + 1) == Some(&b'{') => {
                    if self.i > chunk_start {
                        self.push_tok(Tok::TemplateChunk(&self.src[chunk_start..self.i]));
                    }
                    self.i += 2;
                    self.lex_until_brace()?;
                    chunk_start = self.i;
                }
                b'\n' => {
                    self.line += 1;
                    self.i += 1;
                }
                _ => self.i += 1,
            }
        }
    }

    fn lex_until_brace(&mut self) -> Result<(), String> {
        let base = self.brace_stack.len();
        loop {
            self.skip_ws_and_comments()?;
            if self.i >= self.b.len() {
                return Err(format!("line {}: unterminated expression", self.line));
            }
            if self.b[self.i] == b'}' && self.brace_stack.len() == base {
                self.i += 1;
                return Ok(());
            }
            self.next_token()?;
        }
    }

    fn lex_number(&mut self) {
        let start = self.i;
        if self.b[self.i] == b'0'
            && self
                .b
                .get(self.i + 1)
                .is_some_and(|c| matches!(c, b'x' | b'X' | b'b' | b'B' | b'o' | b'O'))
        {
            self.i += 2;
            while self.i < self.b.len()
                && (self.b[self.i].is_ascii_alphanumeric() || self.b[self.i] == b'_')
            {
                self.i += 1;
            }
        } else {
            while self.i < self.b.len()
                && (self.b[self.i].is_ascii_digit() || self.b[self.i] == b'_')
            {
                self.i += 1;
            }
            if self.b.get(self.i) == Some(&b'.') {
                self.i += 1;
                while self.i < self.b.len()
                    && (self.b[self.i].is_ascii_digit() || self.b[self.i] == b'_')
                {
                    self.i += 1;
                }
            }
            if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
                self.i += 1;
                if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                    self.i += 1;
                }
                while self.i < self.b.len() && self.b[self.i].is_ascii_digit() {
                    self.i += 1;
                }
            }
            if self.b.get(self.i) == Some(&b'n') {
                self.i += 1;
            }
        }
        self.push_tok(Tok::Number(&self.src[start..self.i]));
        self.expr_pos = false;
    }

    fn lex_ident(&mut self) {
        let start = self.i;
        while self.i < self.b.len() && is_ident_continue(self.b[self.i]) {
            if self.b[self.i] >= 0x80 {
                self.i += utf8_len(self.b[self.i]);
            } else {
                self.i += 1;
            }
        }
        let word = &self.src[start..self.i];
        let is_kw =
            JS_KEYWORDS.contains(&word) || (self.dialect.is_ts() && TS_KEYWORDS.contains(&word));
        if is_kw {
            self.push_tok(Tok::Keyword(word));
            self.expr_pos = !VALUE_KEYWORDS.contains(&word);
            if word == "class" {
                self.pending_class = true;
            }
        } else {
            self.push_tok(Tok::Ident(word));
            self.expr_pos = false;
        }
    }

    fn lex_regex(&mut self) -> Result<(), String> {
        let start = self.i;
        self.i += 1;
        let mut in_class = false;
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'\\' => self.i += 2,
                b'[' => {
                    in_class = true;
                    self.i += 1;
                }
                b']' => {
                    in_class = false;
                    self.i += 1;
                }
                b'/' if !in_class => {
                    self.i += 1;
                    while self.i < self.b.len() && self.b[self.i].is_ascii_alphabetic() {
                        self.i += 1;
                    }
                    self.push_tok(Tok::Regex(&self.src[start..self.i]));
                    self.expr_pos = false;
                    return Ok(());
                }
                b'\n' => return Err(format!("line {}: newline in regex", self.line)),
                _ => self.i += 1,
            }
        }
        Err(format!("line {}: unterminated regex", self.line))
    }

    fn lex_punct(&mut self) -> Result<(), String> {
        let rest = &self.src[self.i..];
        let mut matched = "";
        for p in PUNCTUATORS {
            if rest.starts_with(p) {
                matched = p;
                break;
            }
        }
        if matched.is_empty() {
            let c = self.b[self.i];
            if c >= 0x80 {
                let len = utf8_len(c);
                self.push_tok(Tok::Punct(&self.src[self.i..self.i + len]));
                self.i += len;
                self.expr_pos = true;
                return Ok(());
            }
            return Err(format!(
                "line {}: unexpected character `{}`",
                self.line, c as char
            ));
        }
        self.i += matched.len();
        self.push_tok(Tok::Punct(matched));
        match matched {
            "{" => {
                let prev_is_arrow =
                    matches!(self.toks.last().map(|t| &t.0), Some(Tok::Punct("=>")));
                let kind = if self.pending_class {
                    self.pending_class = false;
                    BraceKind::Class
                } else if prev_is_arrow {
                    BraceKind::Block
                } else if self.expr_pos {
                    BraceKind::Object
                } else {
                    BraceKind::Block
                };
                self.brace_stack.push(kind);
                self.brace_kinds.push(kind);
                self.expr_pos = true;
            }
            "}" => {
                let kind = self.brace_stack.pop().unwrap_or(BraceKind::Block);
                self.expr_pos = kind != BraceKind::Object;
            }
            ")" | "]" | "++" | "--" => self.expr_pos = false,
            _ => self.expr_pos = true,
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_lines,
        reason = "JSX text, tags and nested braces share one cursor; the arms are the JSX grammar"
    )]
    fn lex_jsx(&mut self) -> Result<(), String> {
        let mut depth = 0usize;
        loop {
            if self.i >= self.b.len() {
                return Err(format!("line {}: unterminated JSX", self.line));
            }
            match self.b[self.i] {
                b'<' => {
                    if self.b.get(self.i + 1) == Some(&b'/') {
                        self.i += 2;
                        let start = self.i;
                        while self.i < self.b.len() && is_ident_continue(self.b[self.i]) {
                            self.i += 1;
                        }
                        if self.i > start {
                            self.push_tok(Tok::Ident(&self.src[start..self.i]));
                        }
                        while self.i < self.b.len() && self.b[self.i] != b'>' {
                            self.i += 1;
                        }
                        if self.i >= self.b.len() {
                            return Err(format!("line {}: unterminated JSX tag", self.line));
                        }
                        self.i += 1;
                        depth = depth.saturating_sub(1);
                    } else {
                        self.i += 1;
                        let start = self.i;
                        while self.i < self.b.len() && is_ident_continue(self.b[self.i]) {
                            self.i += 1;
                        }
                        if self.i > start {
                            self.push_tok(Tok::Ident(&self.src[start..self.i]));
                        }
                        let mut self_closing = false;
                        while self.i < self.b.len() {
                            match self.b[self.i] {
                                b'>' => {
                                    self.i += 1;
                                    break;
                                }
                                b'/' if self.b.get(self.i + 1) == Some(&b'>') => {
                                    self.i += 2;
                                    self_closing = true;
                                    break;
                                }
                                b'{' => {
                                    self.i += 1;
                                    self.lex_until_brace()?;
                                }
                                b'"' | b'\'' => {
                                    let q = self.b[self.i];
                                    let s = self.i;
                                    self.i += 1;
                                    while self.i < self.b.len() && self.b[self.i] != q {
                                        if self.b[self.i] == b'\\' {
                                            self.i += 1;
                                        }
                                        self.i += 1;
                                    }
                                    self.i += 1;
                                    self.push_tok(Tok::Str(&self.src[s..self.i.min(self.b.len())]));
                                }
                                b'\n' => {
                                    self.line += 1;
                                    self.i += 1;
                                }
                                c if is_ident_start(c) => {
                                    let s = self.i;
                                    while self.i < self.b.len() && is_ident_continue(self.b[self.i])
                                    {
                                        self.i += 1;
                                    }
                                    self.push_tok(Tok::Ident(&self.src[s..self.i]));
                                }
                                b'=' => {
                                    self.i += 1;
                                    self.push_tok(Tok::Punct("="));
                                }
                                b'<' => {
                                    let mut depth = 1usize;
                                    self.i += 1;
                                    while self.i < self.b.len() && depth > 0 {
                                        match self.b[self.i] {
                                            b'<' => depth += 1,
                                            b'>' => depth -= 1,
                                            b'\n' => self.line += 1,
                                            _ => {}
                                        }
                                        self.i += 1;
                                    }
                                }
                                _ => self.i += 1,
                            }
                        }
                        if !self_closing {
                            depth += 1;
                        }
                    }
                }
                b'{' => {
                    self.i += 1;
                    self.lex_until_brace()?;
                }
                b'\n' => {
                    self.line += 1;
                    self.i += 1;
                }
                _ => {
                    let start = self.i;
                    while self.i < self.b.len() && self.b[self.i] != b'<' && self.b[self.i] != b'{'
                    {
                        if self.b[self.i] == b'\n' {
                            self.line += 1;
                        }
                        self.i += 1;
                    }
                    let text = &self.src[start..self.i];
                    if !text.trim().is_empty() {
                        self.push_tok(Tok::JsxText(text));
                    }
                }
            }
            if depth == 0 {
                self.expr_pos = false;
                return Ok(());
            }
        }
    }
}

fn analyze_script(src: &str, dialect: Dialect) -> Result<Vec<RawFunction>, String> {
    let mut lexer = Lexer::new(src, dialect);
    lexer.run()?;
    let toks = lexer.toks;
    let comment_lines = lexer.comment_lines;
    let brace_kinds = lexer.brace_kinds;
    let mut raws = Vec::new();
    let mut i = 0usize;
    let mut brace_stack: Vec<BraceKind> = Vec::new();
    let mut brace_idx = 0usize;
    let mut class_names: Vec<String> = Vec::new();
    let mut pending_class_name: Option<String> = None;
    let type_mask = type_regions(&toks, &brace_kinds);

    while i < toks.len() {
        match &toks[i].0 {
            Tok::Punct("{") => {
                let kind = brace_kinds
                    .get(brace_idx)
                    .copied()
                    .unwrap_or(BraceKind::Block);
                brace_idx += 1;
                if kind == BraceKind::Class {
                    class_names.push(pending_class_name.take().unwrap_or_default());
                }
                brace_stack.push(kind);
            }
            Tok::Punct("}") => {
                if brace_stack.pop() == Some(BraceKind::Class) {
                    class_names.pop();
                }
            }
            _ if type_mask[i] => {}
            Tok::Keyword("class") => {
                if let Some((Tok::Ident(name), _)) = toks.get(i + 1) {
                    pending_class_name = Some((*name).to_string());
                }
            }
            Tok::Keyword("function") => {
                if let Some((raw, _next)) = parse_function(&toks, &comment_lines, i, &class_names) {
                    raws.push(raw);
                }
            }
            Tok::Punct("=>") => {
                if let Some((raw, _next)) = parse_arrow(&toks, &comment_lines, i, &class_names) {
                    raws.push(raw);
                }
            }
            Tok::Ident(_) | Tok::Keyword(_) | Tok::Punct("*" | "[") => {
                let in_class = brace_stack.last() == Some(&BraceKind::Class);
                let in_object = brace_stack.last() == Some(&BraceKind::Object);
                if (in_class || in_object)
                    && let Some((name_idx, name)) = method_name_at(&toks, i)
                    && let Some((raw, _next)) =
                        parse_method(&toks, &comment_lines, name_idx, &name, &class_names)
                {
                    raws.push(raw);
                }
            }
            _ => {}
        }
        i += 1;
    }
    Ok(raws)
}

const METHOD_MODIFIERS: &[&str] = &[
    "static",
    "async",
    "get",
    "set",
    "public",
    "private",
    "protected",
    "readonly",
    "abstract",
    "override",
    "declare",
];

fn method_name_at(toks: &[(Tok, usize)], i: usize) -> Option<(usize, String)> {
    let mut j = i;
    let mut last_modifier: Option<(usize, String)> = None;
    while j < toks.len() {
        match &toks[j].0 {
            Tok::Keyword(k) if METHOD_MODIFIERS.contains(k) => {
                last_modifier = Some((j, (*k).to_string()));
                j += 1;
            }
            Tok::Punct("*") => j += 1,
            _ => break,
        }
    }
    match toks.get(j) {
        Some((Tok::Ident(name), _)) if matches!(toks.get(j + 1), Some((Tok::Punct("("), _))) => {
            Some((j, (*name).to_string()))
        }
        Some((Tok::Punct("("), _)) => last_modifier,
        Some((Tok::Punct("["), _)) => {
            let end = find_matching(toks, j, "[", "]")?;
            if !matches!(toks.get(end + 1), Some((Tok::Punct("("), _))) {
                return None;
            }
            let name: String = toks[j + 1..end]
                .iter()
                .map(|t| match &t.0 {
                    Tok::Ident(n) => (*n).to_string(),
                    Tok::Punct(p) => (*p).to_string(),
                    _ => String::new(),
                })
                .collect();
            Some((end, format!("[{name}]")))
        }
        _ => None,
    }
}

fn at_statement_start(toks: &[(Tok, usize)], i: usize) -> bool {
    match i.checked_sub(1).and_then(|k| toks.get(k)) {
        None | Some((Tok::Punct(";" | "}") | Tok::Keyword("export" | "import"), _)) => true,
        Some((prev, prev_line)) => {
            let cur_line = toks[i].1;
            *prev_line < cur_line
                && matches!(
                    prev,
                    Tok::Ident(_)
                        | Tok::Str(_)
                        | Tok::Number(_)
                        | Tok::Regex(_)
                        | Tok::Punct(")" | "]" | "++" | "--")
                )
        }
    }
}

fn is_statement_starter(k: &str) -> bool {
    matches!(
        k,
        "function"
            | "class"
            | "const"
            | "let"
            | "var"
            | "export"
            | "import"
            | "interface"
            | "type"
            | "enum"
            | "declare"
            | "if"
            | "for"
            | "while"
            | "return"
            | "switch"
            | "try"
            | "throw"
            | "do"
            | "break"
            | "continue"
    )
}

#[expect(
    clippy::too_many_lines,
    reason = "one pass masking TypeScript type-only regions; the nested match mirrors the declaration grammar"
)]
fn type_regions(toks: &[(Tok, usize)], brace_kinds: &[BraceKind]) -> Vec<bool> {
    let mut mask = vec![false; toks.len()];
    let mut i = 0usize;
    while i < toks.len() {
        match &toks[i].0 {
            Tok::Keyword("type" | "declare") if at_statement_start(toks, i) => {
                let mut depth = 0i32;
                let mut j = i + 1;
                while j < toks.len() {
                    match &toks[j].0 {
                        Tok::Punct("{" | "(" | "[" | "<") => {
                            depth += 1;
                        }
                        Tok::Punct("}" | ")" | "]" | ">") => {
                            depth -= 1;
                            if depth < 0 {
                                break;
                            }
                        }
                        Tok::Punct(">>") => depth -= 2,
                        Tok::Punct(">>>") => depth -= 3,
                        Tok::Punct(";") if depth == 0 => {
                            j += 1;
                            break;
                        }
                        Tok::Keyword(k) if depth == 0 && is_statement_starter(k) => break,
                        _ => {}
                    }
                    j += 1;
                }
                for slot in mask.iter_mut().take(j.min(toks.len())).skip(i) {
                    *slot = true;
                }
                i = j;
                continue;
            }
            Tok::Keyword("interface") if at_statement_start(toks, i) => {
                let mut j = i + 1;
                while j < toks.len() && !matches!(toks[j].0, Tok::Punct("{")) {
                    j += 1;
                }
                if j < toks.len()
                    && let Some(end) = find_matching(toks, j, "{", "}")
                {
                    for slot in mask.iter_mut().take(end + 1).skip(i) {
                        *slot = true;
                    }
                    i = end + 1;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let mut brace_stack: Vec<BraceKind> = Vec::new();
    let mut brace_idx = 0usize;
    let mut ternary_depths: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while i < toks.len() {
        match &toks[i].0 {
            Tok::Punct("{") => {
                brace_stack.push(
                    brace_kinds
                        .get(brace_idx)
                        .copied()
                        .unwrap_or(BraceKind::Block),
                );
                brace_idx += 1;
            }
            Tok::Punct("}") => {
                brace_stack.pop();
            }
            Tok::Punct("?") => ternary_depths.push(brace_stack.len()),
            Tok::Punct(":") => {
                let in_object = brace_stack.last() == Some(&BraceKind::Object);
                let is_ternary = ternary_depths.last() == Some(&brace_stack.len());
                if is_ternary {
                    ternary_depths.pop();
                }
                if !in_object && !is_ternary && !mask[i] {
                    let end = annotation_end(toks, i);
                    for slot in mask.iter_mut().take(end).skip(i) {
                        *slot = true;
                    }
                    i = end;
                    continue;
                }
            }
            Tok::Keyword("as" | "satisfies") => {
                let end = annotation_end(toks, i);
                for slot in mask.iter_mut().take(end).skip(i) {
                    *slot = true;
                }
                i = end;
                continue;
            }
            Tok::Keyword("extends") if !brace_stack.is_empty() || in_angle(toks, i) => {
                let end = annotation_end(toks, i);
                for slot in mask.iter_mut().take(end).skip(i) {
                    *slot = true;
                }
                i = end;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    mask
}

fn in_angle(toks: &[(Tok, usize)], at: usize) -> bool {
    let mut depth = 0i32;
    for (tok, _) in toks.iter().take(at).rev() {
        match tok {
            Tok::Punct(">") => depth += 1,
            Tok::Punct("<") => {
                if depth == 0 {
                    return true;
                }
                depth -= 1;
            }
            Tok::Punct(";" | "{" | "}") if depth == 0 => return false,
            _ => {}
        }
    }
    false
}

fn annotation_end(toks: &[(Tok, usize)], start: usize) -> usize {
    let mut k = start + 1;
    let mut depth = 0i32;
    while k < toks.len() {
        match &toks[k].0 {
            Tok::Punct("{") if depth == 0 => {
                let prev = k.checked_sub(1).and_then(|p| toks.get(p)).map(|t| &t.0);
                let in_type_position = matches!(
                    prev,
                    Some(Tok::Punct(
                        ":" | "|" | "&" | "<" | "(" | "," | "=" | "=>" | "["
                    ))
                );
                if in_type_position && let Some(end) = find_matching(toks, k, "{", "}") {
                    k = end + 1;
                    continue;
                }
                return k;
            }
            Tok::Punct("{" | "(" | "[" | "<") => depth += 1,
            Tok::Punct("}" | ")" | "]" | ">") => {
                if depth == 0 {
                    return k;
                }
                depth -= 1;
            }
            Tok::Punct(">>") => {
                if depth <= 2 {
                    return k;
                }
                depth -= 2;
            }
            Tok::Punct(">>>") => {
                if depth <= 3 {
                    return k;
                }
                depth -= 3;
            }
            Tok::Punct("=" | "," | ";" | "=>") if depth == 0 => {
                return k;
            }
            _ => {}
        }
        k += 1;
    }
    k
}
fn find_matching(
    toks: &[(Tok, usize)],
    open: usize,
    open_ch: &str,
    close_ch: &str,
) -> Option<usize> {
    let mut depth = 0usize;
    for (idx, (tok, _)) in toks.iter().enumerate().skip(open) {
        if let Tok::Punct(p) = tok {
            if *p == open_ch {
                depth += 1;
            } else if *p == close_ch {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
        }
    }
    None
}

fn parse_function<'a>(
    toks: &'a [(Tok<'a>, usize)],
    comment_lines: &[bool],
    start: usize,
    class_names: &[String],
) -> Option<(RawFunction, usize)> {
    let mut j = start + 1;
    if matches!(toks.get(j), Some((Tok::Punct("*"), _))) {
        j += 1;
    }
    let name = match toks.get(j) {
        Some((Tok::Ident(n), _)) => (*n).to_string(),
        _ => "<anonymous>".to_string(),
    };
    if matches!(toks.get(j), Some((Tok::Ident(_), _))) {
        j += 1;
    }
    if !matches!(toks.get(j), Some((Tok::Punct("("), _))) {
        return None;
    }
    let params_end = find_matching(toks, j, "(", ")")?;
    let params = count_params(&toks[j + 1..params_end]);
    let mut k = params_end + 1;
    let mut has_return_type = false;
    if matches!(toks.get(k), Some((Tok::Punct(":"), _))) {
        has_return_type = true;
        k = annotation_end(toks, k);
    }
    if !matches!(toks.get(k), Some((Tok::Punct("{"), _))) {
        return None;
    }
    let body_open = k;
    let body_end = find_matching(toks, body_open, "{", "}")?;
    let raw = build_raw(
        toks,
        comment_lines,
        body_open + 1,
        body_end,
        toks[start].1,
        toks[body_end].1,
        &name,
        params + usize::from(has_return_type),
        class_names,
    );
    Some((raw, body_end + 1))
}

#[expect(
    clippy::too_many_lines,
    reason = "reconstructs an arrow function's parameters, body and class owner in one backward-then-forward walk over the tokens"
)]
fn parse_arrow<'a>(
    toks: &'a [(Tok<'a>, usize)],
    comment_lines: &[bool],
    arrow: usize,
    class_names: &[String],
) -> Option<(RawFunction, usize)> {
    let (params, start_idx) = {
        let mut limit = arrow;
        let mut k = arrow;
        let mut depth = 0i32;
        while k > 0 {
            k -= 1;
            match &toks[k].0 {
                Tok::Punct(")" | "]" | ">") => depth += 1,
                Tok::Punct("(") if depth == 0 => break,
                Tok::Punct("(" | "[" | "<") => depth -= 1,
                Tok::Punct(":") if depth == 0 => {
                    if matches!(toks.get(k.wrapping_sub(1)), Some((Tok::Punct(")"), _))) {
                        limit = k;
                        break;
                    }
                }
                Tok::Punct("=" | "," | ";" | "{" | "}") if depth == 0 => {
                    break;
                }
                _ => {}
            }
        }
        let mut k = limit;
        let mut depth = 0i32;
        let mut found = None;
        while k > 0 {
            k -= 1;
            match &toks[k].0 {
                Tok::Punct(")") => {
                    if depth == 0 {
                        found = Some(k);
                        break;
                    }
                    depth += 1;
                }
                Tok::Punct("(") if depth == 0 => break,
                Tok::Punct("(" | "[" | "<") => depth -= 1,
                Tok::Punct("]" | ">") => depth += 1,
                Tok::Ident(_) if depth == 0 => {
                    found = Some(k);
                    break;
                }
                Tok::Punct("=" | "," | ";" | "{" | "}") if depth == 0 => {
                    break;
                }
                _ => {}
            }
        }
        let end = found?;
        if matches!(toks.get(end), Some((Tok::Punct(")"), _))) {
            let mut depth = 0usize;
            let mut open = None;
            for idx in (0..=end).rev() {
                if let Tok::Punct(p) = &toks[idx].0 {
                    if *p == ")" {
                        depth += 1;
                    } else if *p == "(" {
                        depth -= 1;
                        if depth == 0 {
                            open = Some(idx);
                            break;
                        }
                    }
                }
            }
            let open = open?;
            let annotated = matches!(
                toks.get(open.wrapping_sub(1)).map(|t| &t.0),
                Some(Tok::Punct(":" | "?" | "<"))
            );
            if annotated {
                let type_body = {
                    let body_tok = toks.get(arrow + 1).map(|t| &t.0);
                    let after_tok = toks.get(arrow + 2).map(|t| &t.0);
                    let keyword_type =
                        matches!(
                            body_tok,
                            Some(Tok::Keyword("void" | "any" | "unknown" | "never"))
                        ) && matches!(after_tok, Some(Tok::Punct("," | ")" | ";")) | None);
                    let ident_type_closed = matches!(body_tok, Some(Tok::Ident(_)))
                        && matches!(after_tok, Some(Tok::Punct(")")));
                    keyword_type || ident_type_closed
                };
                if type_body {
                    return None;
                }
            }
            (count_params(&toks[open + 1..end]), open)
        } else {
            (1, end)
        }
    };
    let name = {
        let mut k = start_idx;
        if k >= 1 && matches!(toks.get(k - 1), Some((Tok::Punct("="), _))) {
            k -= 1;
        }
        match toks.get(k.wrapping_sub(1)) {
            Some((Tok::Ident(n), _)) => (*n).to_string(),
            _ => "<arrow>".to_string(),
        }
    };
    let body_start = arrow + 1;
    if matches!(toks.get(body_start), Some((Tok::Punct("{"), _))) {
        let body_end = find_matching(toks, body_start, "{", "}")?;
        let raw = build_raw(
            toks,
            comment_lines,
            body_start + 1,
            body_end,
            toks[start_idx].1,
            toks[body_end].1,
            &name,
            params,
            class_names,
        );
        Some((raw, body_end + 1))
    } else {
        let mut depth = 0usize;
        let mut ternary = 0i32;
        let mut end = body_start;
        while end < toks.len() {
            match &toks[end].0 {
                Tok::Punct(p) if matches!(*p, "(" | "[" | "{") => depth += 1,
                Tok::Punct(p) if matches!(*p, ")" | "]" | "}") => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                Tok::Punct("?") if depth == 0 => ternary += 1,
                Tok::Punct(":") if depth == 0 => {
                    if ternary > 0 {
                        ternary -= 1;
                    } else {
                        break;
                    }
                }
                Tok::Punct(p) if matches!(*p, "," | ";") && depth == 0 => break,
                _ => {}
            }
            end += 1;
        }
        let raw = build_raw(
            toks,
            comment_lines,
            body_start,
            end.saturating_sub(1),
            toks[start_idx].1,
            toks[end.saturating_sub(1)].1,
            &name,
            params,
            class_names,
        );
        Some((raw, end))
    }
}

fn parse_method<'a>(
    toks: &'a [(Tok<'a>, usize)],
    comment_lines: &[bool],
    name_idx: usize,
    name: &str,
    class_names: &[String],
) -> Option<(RawFunction, usize)> {
    let open = name_idx + 1;
    let params_end = find_matching(toks, open, "(", ")")?;
    let params = count_params(&toks[open + 1..params_end]);
    let mut k = params_end + 1;
    let mut has_return_type = false;
    if matches!(toks.get(k), Some((Tok::Punct(":"), _))) {
        has_return_type = true;
        k = annotation_end(toks, k);
    }
    if !matches!(toks.get(k), Some((Tok::Punct("{"), _))) {
        return None;
    }
    let body_open = k;
    let body_end = find_matching(toks, body_open, "{", "}")?;
    let qualified = match class_names.last() {
        Some(owner) if !owner.is_empty() => format!("{owner}::{name}"),
        _ => name.to_string(),
    };
    let raw = build_raw(
        toks,
        comment_lines,
        body_open + 1,
        body_end,
        toks[name_idx].1,
        toks[body_end].1,
        &qualified,
        params + usize::from(has_return_type),
        class_names,
    );
    Some((raw, body_end + 1))
}

fn count_params(toks: &[(Tok, usize)]) -> usize {
    let mut names: Vec<&str> = Vec::new();
    let mut expect = true;
    for (tok, _) in toks {
        match tok {
            Tok::Ident(n) if expect => {
                if !names.contains(n) {
                    names.push(n);
                }
                expect = false;
            }
            Tok::Punct(",") => expect = true,
            _ => {}
        }
    }
    names.len()
}

#[allow(clippy::too_many_arguments)]
fn build_raw(
    toks: &[(Tok<'_>, usize)],
    comment_lines: &[bool],
    body_start: usize,
    body_end: usize,
    start_line: usize,
    end_line: usize,
    name: &str,
    io_params: usize,
    _class_names: &[String],
) -> RawFunction {
    let mut collector = MetricsCollector::new(&JS_SPEC, 0);
    for idx in body_start..body_end.min(toks.len()) {
        let (tok, ln) = &toks[idx];
        match tok {
            Tok::Ident(n) => collector.token(&halmi_lex::Tok::Ident(n), *ln),
            Tok::Number(n)
            | Tok::Str(n)
            | Tok::TemplateChunk(n)
            | Tok::Regex(n)
            | Tok::JsxText(n) => collector.token(&halmi_lex::Tok::Str(n), *ln),
            Tok::Keyword(k) => collector.token(&halmi_lex::Tok::Keyword(k), *ln),
            Tok::Punct(p) => match *p {
                "{" => collector.open_block(*ln),
                "}" => collector.close_block(*ln),
                "(" => {
                    let callee = toks.get(idx.wrapping_sub(1)).map(|t| &t.0);
                    let is_call = match callee {
                        Some(Tok::Ident(_)) => true,
                        Some(Tok::Punct(q)) => *q == ")" || *q == "]",
                        _ => false,
                    };
                    if is_call {
                        let is_method = matches!(
                            toks.get(idx.wrapping_sub(2)).map(|t| &t.0),
                            Some(Tok::Punct(q)) if *q == "."
                        );
                        let extends = matches!(
                            toks.get(idx.wrapping_sub(3)).map(|t| &t.0),
                            Some(Tok::Punct(q)) if *q == ")" || *q == "]"
                        );
                        collector.call(is_method, extends);
                    } else {
                        collector.token(&halmi_lex::Tok::Punct("("), *ln);
                    }
                }
                "[" => {
                    let callee = toks.get(idx.wrapping_sub(1)).map(|t| &t.0);
                    if matches!(callee, Some(Tok::Ident(_)))
                        || matches!(callee, Some(Tok::Punct(q)) if *q == "]" || *q == ")")
                    {
                        collector.index();
                    } else {
                        collector.token(&halmi_lex::Tok::Punct("["), *ln);
                    }
                }
                _ => collector.token(&halmi_lex::Tok::Punct(p), *ln),
            },
        }
    }
    let outcome = collector.finish(io_params);
    let span_lines = count_f64(end_line.saturating_sub(start_line) + 1);
    let comment_count = (start_line..=end_line)
        .filter(|l| comment_lines.get(l - 1).copied().unwrap_or(false))
        .count();
    RawFunction {
        path: String::new(),
        language: halmi_core::report::Language::Other,
        name: name.to_string(),
        start_line: count_u32(start_line),
        end_line: count_u32(end_line),
        counts: outcome.counts,
        cyclomatic: outcome.cyclomatic,
        loc: outcome.loc,
        nesting: outcome.nesting,
        comment_density: Some(count_f64(comment_count) / span_lines.max(1.0)),
        method_chain: outcome.method_chain,
    }
}

const JS_SPEC: halmi_lex::LangSpec = halmi_lex::LangSpec {
    decl_op: "function",
    keyword_operators: &[
        "function",
        "if",
        "else",
        "for",
        "while",
        "do",
        "switch",
        "case",
        "select",
        "return",
        "break",
        "continue",
        "go",
        "defer",
        "chan",
        "map",
        "struct",
        "interface",
        "class",
        "const",
        "let",
        "var",
        "import",
        "export",
        "extends",
        "new",
        "typeof",
        "instanceof",
        "in",
        "of",
        "static",
        "delete",
        "void",
        "yield",
        "try",
        "finally",
        "throw",
        "this",
        "super",
        "true",
        "false",
        "null",
        "async",
        "await",
        "get",
        "set",
    ],
    cc_keywords: &["if", "for", "while", "do", "case", "catch"],
    separator_ops: &["(", ")", "[", "]", "{", "}", ";", ",", ":"],
    cc_ops: &["&&", "||", "??", "?"],
    loc_mode: halmi_lex::LocMode::Physical,
};
