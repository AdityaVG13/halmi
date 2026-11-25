//! halmi-lex: the shared toolkit for halmi language adapters.
//!
//! One place for the cross-language rules so adapters stay thin:
//!
//! * [`MetricsCollector`] turns a token stream into the metric signals
//!   (Halstead counts, cyclomatic complexity, logical LOC, nesting,
//!   method-chain length) using a per-language [`LangSpec`]. The
//!   declaration-operator rule (every function counts exactly one decl
//!   operator) lives here, once.
//! * [`cfamily`] provides the C-family scanning toolkit: `//` and
//!   `/* */` comments with line marking, escaped and raw strings,
//!   numbers, and the C-family operator set with longest-match. Zig, C,
//!   and Swift adapters build on it instead of re-deriving it.
//! * [`density`] computes the fraction of comment lines in a span.
//!
//! Adapters that need language-specific scanning (Python's indentation
//! machine, JavaScript's regex/JSX/template context tracking) keep their
//! own scan loop but feed tokens into [`MetricsCollector`].

#![deny(unsafe_code)]

use halmi_core::HalsteadCounts;

/// A lexical token. The common superset adapters map onto before metric
/// collection; language-specific kinds (JSX text, template chunks, regex
/// literals, Indent/Dedent) stay in their adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok<'a> {
    Ident(&'a str),
    Number(&'a str),
    Str(&'a str),
    Punct(&'a str),
    Keyword(&'a str),
    Newline,
}

/// How logical LOC is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocMode {
    /// Non-empty logical lines delimited by `Tok::Newline` (continuations
    /// and open brackets join lines): Python, Rust-style statement counts.
    Logical,
    /// Distinct physical lines that carry at least one code token:
    /// Go, JavaScript-style line counts.
    Physical,
}

/// The per-language classification rules that drive metric collection.
#[derive(Debug, Clone, Copy)]
pub struct LangSpec {
    /// The single declaration operator every function counts once
    /// (e.g. "fn", "def", "func", "function").
    pub decl_op: &'static str,
    /// Keywords counted as Halstead operators (control + declaration).
    pub keyword_operators: &'static [&'static str],
    /// Keywords that each add one cyclomatic-complexity branch.
    pub cc_keywords: &'static [&'static str],
    /// Punctuators that are separators (never operators).
    pub separator_ops: &'static [&'static str],
    /// Binary-operator punctuators that each add one CC branch.
    pub cc_ops: &'static [&'static str],
    /// How LOC is derived from the token stream.
    pub loc_mode: LocMode,
}

/// Standard separator sets, so adapters do not re-list them.
pub const PAREN_SEPARATORS: &[&str] = &["(", ")", "[", "]", "{", "}", ";", ",", ":"];

/// The collected metric signals for one function body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricsOutcome {
    pub counts: HalsteadCounts,
    pub cyclomatic: u32,
    pub loc: u32,
    pub nesting: u32,
    pub method_chain: u32,
}

/// Collects Halstead/CC/LOC/nesting/chain signals from a token stream.
///
/// The adapter walks the body, calling [`MetricsCollector::token`] for each
/// token, plus the `note_*` methods for structure it tracks itself
/// (brace depth, indent depth, call sites). `finish` produces the
/// outcome; the declaration operator is counted once here per the
/// cross-language rule.
pub struct MetricsCollector<'a> {
    spec: &'a LangSpec,
    base_depth: u32,
    ops: Vec<(&'a str, u32)>,
    operands: Vec<(&'a str, u32)>,
    cc: u32,
    logical_loc: u32,
    in_logical_line: bool,
    code_line_last: Option<usize>,
    code_line_count: u32,
    max_nesting: u32,
    depth: u32,
    cur_chain: u32,
    max_chain: u32,
}

impl<'a> MetricsCollector<'a> {
    #[must_use]
    pub fn new(spec: &'a LangSpec, base_depth: u32) -> Self {
        let mut c = MetricsCollector {
            spec,
            base_depth,
            ops: Vec::with_capacity(16),
            operands: Vec::with_capacity(16),
            cc: 1,
            logical_loc: 0,
            in_logical_line: false,
            code_line_last: None,
            code_line_count: 0,
            max_nesting: base_depth,
            depth: base_depth,
            cur_chain: 0,
            max_chain: 0,
        };
        c.op(spec.decl_op);
        c
    }

    fn op(&mut self, kind: &'a str) {
        for (k, n) in &mut self.ops {
            if *k == kind {
                *n += 1;
                return;
            }
        }
        self.ops.push((kind, 1));
    }

    fn operand(&mut self, text: &'a str) {
        for (k, n) in &mut self.operands {
            if *k == text {
                *n += 1;
                return;
            }
        }
        self.operands.push((text, 1));
    }

    /// Process one token. `line` is its 1-based line number.
    pub fn token(&mut self, tok: &Tok<'a>, line: usize) {
        match tok {
            Tok::Newline => {
                if self.in_logical_line {
                    self.logical_loc += 1;
                    self.in_logical_line = false;
                }
                self.cur_chain = 0;
            }
            Tok::Ident(n) | Tok::Number(n) | Tok::Str(n) => {
                self.operand(n);
                self.in_logical_line = true;
                if self.code_line_last != Some(line) {
                    self.code_line_count += 1;
                    self.code_line_last = Some(line);
                }
            }
            Tok::Keyword(k) => {
                self.in_logical_line = true;
                if self.code_line_last != Some(line) {
                    self.code_line_count += 1;
                    self.code_line_last = Some(line);
                }
                if self.spec.keyword_operators.contains(k) {
                    self.op(k);
                }
                if self.spec.cc_keywords.contains(k) {
                    self.cc += 1;
                }
            }
            Tok::Punct(p) => {
                self.in_logical_line = true;
                if self.code_line_last != Some(line) {
                    self.code_line_count += 1;
                    self.code_line_last = Some(line);
                }
                if self.spec.cc_ops.contains(p) {
                    self.cc += 1;
                }
                if *p == "." {
                    self.op("attr");
                } else if !self.spec.separator_ops.contains(p) {
                    self.op(p);
                }
            }
        }
    }

    /// Record a detected call. `is_method` is true for dotted calls
    /// (`x.method()`). `extends` is true when the receiver is the result
    /// of a previous call (the call 3 tokens back is a closing paren),
    /// continuing a chain. Dotted calls always count as at least 1.
    pub fn call(&mut self, is_method: bool, extends: bool) {
        self.op("call");
        if !is_method {
            self.cur_chain = 0;
        } else if extends {
            self.cur_chain += 1;
        } else {
            self.cur_chain = 1;
        }
        self.max_chain = self.max_chain.max(self.cur_chain);
    }

    /// Record an index (`x[i]`).
    pub fn index(&mut self) {
        self.op("index");
    }

    /// An opening brace: nesting depth +1, records the line for Physical LOC.
    pub fn open_block(&mut self, line: usize) {
        self.depth += 1;
        self.max_nesting = self.max_nesting.max(self.depth);
        if self.code_line_last != Some(line) {
            self.code_line_count += 1;
            self.code_line_last = Some(line);
        }
    }

    /// A closing brace: nesting depth -1, records the line for Physical LOC.
    pub fn close_block(&mut self, line: usize) {
        self.depth = self.depth.saturating_sub(1);
        if self.code_line_last != Some(line) {
            self.code_line_count += 1;
            self.code_line_last = Some(line);
        }
    }

    /// Set the depth directly (for indentation-based adapters).
    pub fn set_depth(&mut self, depth: u32) {
        self.depth = depth;
        self.max_nesting = self.max_nesting.max(depth);
    }

    /// Current depth (for indentation-based adapters that track depth
    /// via Indent/Dedent and need to read it back to increment).
    #[must_use]
    pub const fn current_depth(&self) -> u32 {
        self.depth
    }

    /// Finish and produce the outcome. `io_params` comes from the
    /// adapter's signature scan.
    pub fn finish(&mut self, io_params: usize) -> MetricsOutcome {
        if self.in_logical_line {
            self.logical_loc += 1;
        }
        let loc = match self.spec.loc_mode {
            LocMode::Logical => self.logical_loc,
            LocMode::Physical => self.code_line_count,
        };
        let outcome = MetricsOutcome {
            counts: HalsteadCounts {
                eta1: self.ops.len(),
                eta2: self.operands.len(),
                n1: self.ops.iter().map(|(_, n)| *n as usize).sum(),
                n2: self.operands.iter().map(|(_, n)| *n as usize).sum(),
                io_params,
            },
            cyclomatic: self.cc,
            loc,
            nesting: self.max_nesting,
            method_chain: self.max_chain,
        };
        self.reset();
        outcome
    }

    /// Clear per-function state, retaining map capacity for the next
    /// function in the same file.
    pub fn reset(&mut self) {
        self.ops.clear();
        self.operands.clear();
        self.cc = 1;
        self.logical_loc = 0;
        self.in_logical_line = false;
        self.code_line_last = None;
        self.code_line_count = 0;
        self.cur_chain = 0;
        self.max_chain = 0;
        self.depth = self.base_depth;
        self.max_nesting = self.base_depth;
        self.op(self.spec.decl_op);
    }
}

/// Fraction of comment lines in a 1-based inclusive line span.
#[must_use]
pub fn density(map: &[bool], start_line: u32, end_line: u32) -> f64 {
    let s = (start_line as usize).saturating_sub(1).min(map.len());
    let e = (end_line as usize).min(map.len());
    if e <= s {
        return 0.0;
    }
    let slice = &map[s..e];
    let commented = u32::try_from(slice.iter().filter(|&&c| c).count()).unwrap_or(u32::MAX);
    let total = u32::try_from(slice.len()).unwrap_or(u32::MAX);
    f64::from(commented) / f64::from(total)
}

/// The C-family scanning toolkit: comments, strings, numbers, and the
/// common operator set. Built from the Go and Script adapters; Zig, C,
/// and Swift adapters use these instead of re-deriving them.
pub mod cfamily {
    #[must_use]
    pub const fn utf8_len(first: u8) -> usize {
        match first {
            0x00..=0x7F => 1,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            _ => 4,
        }
    }

    /// Skip past a line comment (`//` to end of line). Returns false at EOF.
    pub const fn skip_line_comment(b: &[u8], i: &mut usize) -> bool {
        while *i < b.len() && b[*i] != b'\n' {
            *i += 1;
        }
        *i < b.len()
    }

    /// Skip a block comment (`/* */`). Advances `i` past it, marks comment
    /// lines, and updates `line`.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the comment is unterminated.
    pub fn skip_block_comment(
        b: &[u8],
        i: &mut usize,
        line: &mut usize,
        n_lines: usize,
        comment_lines: &mut [bool],
    ) -> Result<(), String> {
        let mut depth = 0usize;
        while *i + 1 < b.len() {
            if b[*i] == b'/' && b[*i + 1] == b'*' {
                depth += 1;
                *i += 2;
            } else if b[*i] == b'*' && b[*i + 1] == b'/' {
                depth -= 1;
                *i += 2;
                if depth == 0 {
                    return Ok(());
                }
            } else {
                if b[*i] == b'\n' {
                    *line += 1;
                    if *line <= n_lines {
                        comment_lines[*line - 1] = true;
                    }
                }
                *i += 1;
            }
        }
        Err(format!("line {line}: unterminated block comment"))
    }

    /// Scan an escaped string starting at the opening quote. Returns
    /// (index past closing quote, line unchanged).
    ///
    /// # Errors
    ///
    /// Returns `Err` on a newline inside the string or at end of input.
    pub fn scan_string(b: &[u8], start: usize, line: usize) -> Result<usize, String> {
        let mut i = start + 1;
        while i < b.len() {
            match b[i] {
                b'\\' => i += 2,
                b'"' => return Ok(i + 1),
                b'\n' => return Err(format!("line {line}: newline in string")),
                _ => i += 1,
            }
        }
        Err(format!("line {line}: unterminated string"))
    }

    /// Scan a backtick raw string starting at the opening backtick.
    /// Updates `line` past newlines.
    ///
    /// # Errors
    ///
    /// Returns `Err` at end of input (no closing backtick).
    pub fn scan_raw_string(b: &[u8], start: usize, line: &mut usize) -> Result<usize, String> {
        let mut i = start + 1;
        while i < b.len() && b[i] != b'`' {
            if b[i] == b'\n' {
                *line += 1;
            }
            i += 1;
        }
        if i >= b.len() {
            return Err(format!("line {line}: unterminated raw string"));
        }
        Ok(i + 1)
    }

    /// Scan a number: hex/binary/octal prefixes, decimal, float,
    /// scientific, underscores, `..` range exclusion, bigint `n`.
    #[must_use]
    pub const fn scan_number(b: &[u8], start: usize) -> usize {
        let mut i = start;
        if b[i] == b'0'
            && i + 1 < b.len()
            && matches!(b[i + 1], b'x' | b'X' | b'b' | b'B' | b'o' | b'O')
        {
            i += 2;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            return i;
        }
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
            i += 1;
        }
        if i < b.len() && b[i] == b'.' && i + 1 < b.len() && b[i + 1] != b'.' {
            i += 1;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'_') {
                i += 1;
            }
        }
        if i < b.len() && matches!(b[i], b'e' | b'E') {
            i += 1;
            if i < b.len() && matches!(b[i], b'+' | b'-') {
                i += 1;
            }
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i < b.len() && b[i] == b'n' {
            i += 1;
        }
        i
    }

    /// C-family punctuators, longest first.
    pub const PUNCTUATORS: &[&str] = &[
        ">>>=", "..<", ".?", ".*", "...", "..", "===", "!==", "**=", "&&=", "||=", "??=", ">>>",
        "<<=", ">>=", "=>", "::", "->", "==", "!=", "<=", ">=", "&&", "||", "??", "?.", "++", "--",
        "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "**", "<<", ">>", "(", ")", "{", "}", "[",
        "]", ";", ",", "<", ">", "+", "-", "*", "/", "%", "&", "|", "^", "!", "~", "?", ":", "=",
        ".", "@", "#",
    ];

    /// Longest C-family punctuator match at byte offset `i` in `src`.
    /// Returns the matched length (0 = no punctuator).
    #[must_use]
    pub fn punct_len(src: &str, i: usize) -> usize {
        let b = src.as_bytes();
        let Some(&c) = b.get(i) else {
            return 0;
        };
        match c {
            b'>' | b'<' => punct_len_angle(b, i, c),
            b'.' | b':' | b'-' => punct_len_dot_colon_dash(b, i, c),
            b'=' | b'!' | b'&' | b'|' | b'?' | b'*' | b'+' | b'/' | b'%' | b'^' | b'~' => {
                punct_len_op_assign(b, i, c)
            }
            b'(' | b')' | b'{' | b'}' | b'[' | b']' | b';' | b',' | b'@' | b'#' | b'$' => 1,
            _ => 0,
        }
    }

    /// `>` and `<`: shifts, `>>>`, and their `=` forms.
    fn punct_len_angle(b: &[u8], i: usize, c: u8) -> usize {
        let rest3 = b.get(i + 3).is_some_and(|&x| x == b'=');
        match c {
            b'>' => {
                if b[i..].starts_with(b">>>=") {
                    4
                } else if b[i..].starts_with(b">>>") {
                    3
                } else if rest3 && b.get(i + 1) == Some(&b'>') && b.get(i + 2) == Some(&b'>') {
                    4
                } else if b[i..].starts_with(b">>=") {
                    3
                } else if b.get(i + 1) == Some(&b'>') || b.get(i + 1) == Some(&b'=') {
                    2
                } else {
                    1
                }
            }
            _ => {
                if b[i..].starts_with(b"<<=") {
                    3
                } else if b.get(i + 1) == Some(&b'<') || b.get(i + 1) == Some(&b'=') {
                    2
                } else {
                    1
                }
            }
        }
    }

    /// `.`, `:` and `-`: ranges, `?.`, `.*`, `::`, `->`, `--`, `-=`.
    fn punct_len_dot_colon_dash(b: &[u8], i: usize, c: u8) -> usize {
        match c {
            b'.' => {
                if b.get(i + 1) == Some(&b'.') {
                    match b.get(i + 2) {
                        Some(&b'<' | &b'.') => 3,
                        _ => 2,
                    }
                } else if b.get(i + 1) == Some(&b'?') || b.get(i + 1) == Some(&b'*') {
                    2
                } else {
                    1
                }
            }
            b':' => {
                if b.get(i + 1) == Some(&b':') {
                    2
                } else {
                    1
                }
            }
            _ => {
                if b[i..].starts_with(b"->")
                    || b.get(i + 1) == Some(&b'-')
                    || b.get(i + 1) == Some(&b'=')
                {
                    2
                } else {
                    1
                }
            }
        }
    }

    /// `=` `!` `&` `|` `?` `*` `+` `/` `%` `^` `~`: comparisons, logical
    /// operators and compound assignments (`===`, `!==`, `&&=`, `||=`,
    /// `??=`, `**=`, `x=`).
    fn punct_len_op_assign(b: &[u8], i: usize, c: u8) -> usize {
        let rest2 = b.get(i + 2).is_some_and(|&x| x == b'=');
        match c {
            b'=' => {
                if b[i..].starts_with(b"===") {
                    3
                } else if b.get(i + 1) == Some(&b'=') || b.get(i + 1) == Some(&b'>') {
                    2
                } else {
                    1
                }
            }
            b'!' => {
                if b[i..].starts_with(b"!==") {
                    3
                } else if b.get(i + 1) == Some(&b'=') {
                    2
                } else {
                    1
                }
            }
            b'&' => {
                if b[i..].starts_with(b"&&=") {
                    3
                } else if b.get(i + 1) == Some(&b'&') {
                    2
                } else if rest2 && b.get(i + 1) == Some(&b'&') {
                    3
                } else if rest2 {
                    2
                } else {
                    1
                }
            }
            b'|' => {
                if b[i..].starts_with(b"||=") {
                    3
                } else if b.get(i + 1) == Some(&b'|') || rest2 {
                    2
                } else {
                    1
                }
            }
            b'?' => {
                if b[i..].starts_with(b"??=") {
                    3
                } else if b[i..].starts_with(b"?.") || b.get(i + 1) == Some(&b'?') {
                    2
                } else {
                    1
                }
            }
            b'*' => {
                if b[i..].starts_with(b"**=") {
                    3
                } else if b.get(i + 1) == Some(&b'*') || rest2 {
                    2
                } else {
                    1
                }
            }
            b'+' => {
                if b.get(i + 1) == Some(&b'+') || b.get(i + 1) == Some(&b'=') {
                    2
                } else {
                    1
                }
            }
            _ => {
                if rest2 {
                    2
                } else {
                    1
                }
            }
        }
    }
}
