//! Minimal, dependency-free JSON support for halmi's own schema.
//!
//! The writer always emits deterministic, valid JSON for the shapes halmi
//! serializes (objects, arrays, strings, finite numbers, null, booleans).
//! The reader is a small recursive-descent parser over the same subset,
//! sufficient for halmi baselines and any agent-side JSON of this schema;
//! unknown fields are preserved so future schema versions still load.

use std::fmt::Write as _;

/// Append a JSON string literal (quoted, escaped) to `out`.
pub fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Append a finite f64. Non-finite values (never produced by the metric
/// formulas) are written as `0`.
pub fn write_f64(out: &mut String, v: f64) {
    if v.is_finite() {
        let _ = write!(out, "{v}");
    } else {
        out.push('0');
    }
}

/// A parsed JSON value. Objects preserve insertion order.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Self>),
    Obj(Vec<(String, Self)>),
}

impl Json {
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Num(n) => Some(*n),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_u32(&self) -> Option<u32> {
        self.as_f64().and_then(|n| {
            if n >= 0.0 && n.fract() == 0.0 && n <= f64::from(u32::MAX) {
                #[expect(
                    clippy::cast_sign_loss,
                    clippy::cast_possible_truncation,
                    reason = "range-checked above; f64 has no TryFrom<u32>"
                )]
                Some(n as u32)
            } else {
                None
            }
        })
    }

    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Object field lookup.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_object(&self) -> Option<&Vec<(String, Self)>> {
        match self {
            Self::Obj(fields) => Some(fields),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Arr(items) => Some(items),
            _ => None,
        }
    }
}

impl From<&str> for Json {
    fn from(s: &str) -> Self {
        Self::Str(s.to_string())
    }
}

/// Parse a JSON document. Errors carry a short message with the byte offset.
///
/// # Errors
///
/// Returns `Err` for malformed input: bad tokens, unterminated strings,
/// trailing data after the top-level value, or numbers out of range.
pub fn parse(text: &str) -> Result<Json, String> {
    let mut p = Parser {
        b: text.as_bytes(),
        pos: 0,
    };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.pos != p.b.len() {
        return Err(p.err("trailing bytes after value"));
    }
    Ok(v)
}

struct Parser<'a> {
    b: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn err(&self, why: &str) -> String {
        format!("json: {why} at byte {}", self.pos)
    }

    const fn ws(&mut self) {
        while self.pos < self.b.len() && matches!(self.b[self.pos], b' ' | b'\t' | b'\n' | b'\r') {
            self.pos += 1;
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        match self.b.get(self.pos) {
            None => Err(self.err("unexpected end")),
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'n') => self.lit("null", Json::Null),
            Some(c) if c.is_ascii_digit() || *c == b'-' => self.number(),
            Some(_) => Err(self.err("unexpected byte")),
        }
    }

    fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.b[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(v)
        } else {
            Err(self.err("invalid literal"))
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.pos += 1;
        let mut fields = Vec::new();
        self.ws();
        if self.b.get(self.pos) == Some(&b'}') {
            self.pos += 1;
            return Ok(Json::Obj(fields));
        }
        loop {
            self.ws();
            if self.b.get(self.pos) != Some(&b'"') {
                return Err(self.err("expected object key"));
            }
            let key = self.string()?;
            self.ws();
            if self.b.get(self.pos) != Some(&b':') {
                return Err(self.err("expected ':'"));
            }
            self.pos += 1;
            self.ws();
            let v = self.value()?;
            fields.push((key, v));
            self.ws();
            match self.b.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Obj(fields));
                }
                _ => return Err(self.err("expected ',' or '}'")),
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.pos += 1;
        let mut items = Vec::new();
        self.ws();
        if self.b.get(self.pos) == Some(&b']') {
            self.pos += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            self.ws();
            items.push(self.value()?);
            self.ws();
            match self.b.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Arr(items));
                }
                _ => return Err(self.err("expected ',' or ']'")),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.pos += 1;
        let mut s = String::new();
        loop {
            match self.b.get(self.pos) {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(s);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.b.get(self.pos) {
                        Some(b'"') => {
                            s.push('"');
                            self.pos += 1;
                        }
                        Some(b'\\') => {
                            s.push('\\');
                            self.pos += 1;
                        }
                        Some(b'/') => {
                            s.push('/');
                            self.pos += 1;
                        }
                        Some(b'n') => {
                            s.push('\n');
                            self.pos += 1;
                        }
                        Some(b't') => {
                            s.push('\t');
                            self.pos += 1;
                        }
                        Some(b'r') => {
                            s.push('\r');
                            self.pos += 1;
                        }
                        Some(b'b') => {
                            s.push('\u{08}');
                            self.pos += 1;
                        }
                        Some(b'f') => {
                            s.push('\u{0c}');
                            self.pos += 1;
                        }
                        Some(b'u') => {
                            self.pos += 1;
                            let high = self.hex4()?;
                            let c = if (0xD800..0xDC00).contains(&high) {
                                if self.b.get(self.pos) == Some(&b'\\')
                                    && self.b.get(self.pos + 1) == Some(&b'u')
                                {
                                    self.pos += 2;
                                    let low = self.hex4()?;
                                    if !(0xDC00..0xE000).contains(&low) {
                                        return Err(self.err("bad surrogate pair"));
                                    }
                                    let cp = 0x10000
                                        + (u32::from(high - 0xD800) << 10)
                                        + u32::from(low - 0xDC00);
                                    char::from_u32(cp).ok_or_else(|| self.err("bad surrogate"))?
                                } else {
                                    return Err(self.err("unpaired surrogate"));
                                }
                            } else {
                                char::from_u32(u32::from(high))
                                    .ok_or_else(|| self.err("bad \\u escape"))?
                            };
                            s.push(c);
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
                Some(&c) if c < 0x20 => return Err(self.err("raw control char in string")),
                Some(_) => {
                    let start = self.pos;
                    let len = utf8_len(self.b[self.pos]);
                    self.pos += len;
                    let chunk = self
                        .b
                        .get(start..self.pos)
                        .ok_or_else(|| self.err("truncated utf-8"))?;
                    s.push_str(std::str::from_utf8(chunk).map_err(|_| self.err("bad utf-8"))?);
                }
            }
        }
    }

    /// Read exactly 4 hex digits at `pos` and advance past them.
    fn hex4(&mut self) -> Result<u16, String> {
        let end = self.pos + 4;
        let hex = self
            .b
            .get(self.pos..end)
            .ok_or_else(|| self.err("truncated \\u escape"))?;
        let s = std::str::from_utf8(hex).map_err(|_| self.err("bad \\u escape"))?;
        self.pos = end;
        u16::from_str_radix(s, 16).map_err(|_| self.err("bad \\u escape"))
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        if self.b.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        match self.b.get(self.pos) {
            Some(b'0') => {
                self.pos += 1;
                if matches!(self.b.get(self.pos), Some(c) if c.is_ascii_digit()) {
                    return Err(self.err("leading zero in number"));
                }
            }
            Some(b'1'..=b'9') => {
                self.pos += 1;
                while matches!(self.b.get(self.pos), Some(c) if c.is_ascii_digit()) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.err("expected digit")),
        }
        if self.b.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            if !matches!(self.b.get(self.pos), Some(c) if c.is_ascii_digit()) {
                return Err(self.err("expected digit after decimal point"));
            }
            while matches!(self.b.get(self.pos), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.b.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.b.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.b.get(self.pos), Some(c) if c.is_ascii_digit()) {
                return Err(self.err("expected exponent digits"));
            }
            while matches!(self.b.get(self.pos), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let text =
            std::str::from_utf8(&self.b[start..self.pos]).map_err(|_| self.err("bad number"))?;
        let value = text.parse::<f64>().map_err(|_| self.err("bad number"))?;
        if !value.is_finite() {
            return Err(self.err("number is outside finite f64 range"));
        }
        Ok(Json::Num(value))
    }
}

const fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}
