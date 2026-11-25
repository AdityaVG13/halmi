//! A conservative line scanner that marks which lines of a source file
//! contain Rust comments (line or block, including nested block comments).
//!
//! It tracks string, byte-string, raw-string, char, and byte-char literals so
//! comment markers inside them are ignored, and lifetimes so `'a` is not
//! mistaken for an unterminated char literal.

/// `map[i]` is true when line `i + 1` contains comment text.
pub fn comment_line_map(src: &str) -> Vec<bool> {
    let mut scanner = Scanner::new(src);
    scanner.run();
    scanner.map
}

enum State {
    Code,
    LineComment,
    BlockComment,
    Str,
    RawStr(usize),
}

struct Scanner<'a> {
    b: &'a [u8],
    i: usize,
    line: usize,
    map: Vec<bool>,
    block_depth: usize,
    state: State,
}

impl<'a> Scanner<'a> {
    fn new(src: &'a str) -> Self {
        Scanner {
            b: src.as_bytes(),
            i: 0,
            line: 0,
            map: vec![false; src.lines().count().max(1)],
            block_depth: 0,
            state: State::Code,
        }
    }

    fn run(&mut self) {
        while self.i < self.b.len() {
            match self.state {
                State::Code => self.step_code(),
                State::LineComment => self.step_line_comment(),
                State::BlockComment => self.step_block_comment(),
                State::Str => self.step_str(),
                State::RawStr(hashes) => self.step_raw_str(hashes),
            }
        }
    }

    fn step_code(&mut self) {
        let c = self.b[self.i];
        match c {
            b'\n' => {
                self.line += 1;
                self.i += 1;
            }
            b'/' if self.i + 1 < self.b.len() && self.b[self.i + 1] == b'/' => {
                self.mark();
                self.state = State::LineComment;
                self.i += 2;
            }
            b'/' if self.i + 1 < self.b.len() && self.b[self.i + 1] == b'*' => {
                self.mark();
                self.block_depth = 1;
                self.state = State::BlockComment;
                self.i += 2;
            }
            b'"' => {
                self.state = State::Str;
                self.i += 1;
            }
            b'\'' => self.step_char_literal(),
            b'r' | b'b' => self.step_string_prefix(),
            _ => self.i += 1,
        }
    }

    fn mark(&mut self) {
        if self.line < self.map.len() {
            self.map[self.line] = true;
        }
    }

    const fn step_char_literal(&mut self) {
        let b = self.b;
        let i = self.i;
        if i + 1 < b.len() && b[i + 1] == b'\\' {
            self.i += 2;
            if self.i < b.len() && b[self.i] == b'\'' {
                self.i += 1;
            }
            while self.i < b.len() && b[self.i] != b'\'' {
                if b[self.i] == b'\n' {
                    self.line += 1;
                }
                self.i += 1;
            }
            self.i += 1;
        } else if i + 2 < b.len() && b[i + 2] == b'\'' && b[i + 1] != b'\'' {
            self.i += 3;
        } else {
            self.i += 1;
        }
    }

    const fn step_string_prefix(&mut self) {
        match string_prefix_len(self.b, self.i) {
            Some((raw, hashes, len)) => {
                if raw {
                    self.state = State::RawStr(hashes);
                } else if hashes == 0 && self.b[self.i + len - 1] != b'"' {
                    self.state = State::Code;
                } else {
                    self.state = State::Str;
                }
                self.i += len;
            }
            None => self.i += 1,
        }
    }

    const fn step_line_comment(&mut self) {
        if self.b[self.i] == b'\n' {
            self.line += 1;
            self.state = State::Code;
        }
        self.i += 1;
    }

    fn step_block_comment(&mut self) {
        let b = self.b;
        let c = b[self.i];
        if c == b'/' && self.i + 1 < b.len() && b[self.i + 1] == b'*' {
            self.block_depth += 1;
            self.mark();
            self.i += 2;
        } else if c == b'*' && self.i + 1 < b.len() && b[self.i + 1] == b'/' {
            self.block_depth -= 1;
            self.i += 2;
            if self.block_depth == 0 {
                self.state = State::Code;
            }
        } else {
            if c == b'\n' {
                self.line += 1;
                self.mark();
            }
            self.i += 1;
        }
    }

    const fn step_str(&mut self) {
        let b = self.b;
        match b[self.i] {
            b'\\' => {
                self.i += 2;
                if self.i - 1 < b.len() && b[self.i - 1] == b'\n' {
                    self.line += 1;
                }
            }
            b'"' => {
                self.state = State::Code;
                self.i += 1;
            }
            b'\n' => {
                self.line += 1;
                self.i += 1;
            }
            _ => self.i += 1,
        }
    }

    fn step_raw_str(&mut self, hashes: usize) {
        let b = self.b;
        if b[self.i] == b'"' {
            let mut j = self.i + 1;
            let mut ok = true;
            for _ in 0..hashes {
                if j >= b.len() || b[j] != b'#' {
                    ok = false;
                    break;
                }
                j += 1;
            }
            if ok {
                self.i = j;
                self.state = State::Code;
                return;
            }
        }
        if b[self.i] == b'\n' {
            self.line += 1;
        }
        self.i += 1;
    }
}

/// At `i` pointing at an `r`/`b`, try to parse a string prefix:
/// `[b]r #* "` (raw, returns hash count) or `b"` / `b'`.
/// Returns `(is_raw, hashes, prefix_len)`.
const fn string_prefix_len(b: &[u8], i: usize) -> Option<(bool, usize, usize)> {
    let mut j = i;
    if b[j] == b'b' {
        j += 1;
    }
    let has_r = j < b.len() && b[j] == b'r';
    if has_r {
        j += 1;
    }
    if j >= b.len() {
        return None;
    }
    if has_r {
        let mut hashes = 0usize;
        while j < b.len() && b[j] == b'#' {
            hashes += 1;
            j += 1;
        }
        if j < b.len() && b[j] == b'"' {
            return Some((true, hashes, j + 1 - i));
        }
    } else if b[j] == b'"' {
        return Some((false, 0, j + 1 - i));
    } else if b[j] == b'\'' {
        let mut k = j + 1;
        if k < b.len() && b[k] == b'\\' {
            k += 2;
            while k < b.len() && b[k] != b'\'' {
                k += 1;
            }
            k += 1;
        } else {
            k += 2;
        }
        return Some((false, 0, k - i));
    }
    None
}

/// Fraction of comment lines in the 1-based inclusive line span.
/// `None` when the span is empty or outside the file.
pub fn density(map: &[bool], start_line: u32, end_line: u32) -> Option<f64> {
    if end_line < start_line || map.is_empty() {
        return None;
    }
    let s = (start_line as usize).saturating_sub(1);
    let e = (end_line as usize).min(map.len());
    if s >= e {
        return None;
    }
    let slice = &map[s..e];
    let commented = u32::try_from(slice.iter().filter(|&&c| c).count()).unwrap_or(u32::MAX);
    let total = u32::try_from(slice.len()).unwrap_or(u32::MAX);
    Some(f64::from(commented) / f64::from(total))
}
