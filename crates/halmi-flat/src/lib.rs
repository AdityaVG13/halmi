//! halmi-flat: the minimal reference adapter, proving that a new language
//! is a small crate implementing `halmi_core::adapter::LanguageAdapter`.
//!
//! flat-script grammar (line-based, indentation-scoped):
//!
//! ```text
//! # a comment line
//! def name:
//!     operand + operand - operand
//!     if operand < operand
//! ```
//!
//! * A function starts at a `def NAME:` line and ends before the next line
//!   whose indentation is <= the `def` line's.
//! * Operators: the symbol tokens `+ - * / % < > =` (by symbol text).
//! * Operands: word tokens that are not `def` or `if` (by text).
//! * Cyclomatic complexity: 1 + occurrences of the word `if`.
//! * Logical LOC: body lines (comments excluded).
//! * Comment density: fraction of `#` lines in the span; method chains: 0
//!   (the language has none -- the contract lets adapters report 0).
//! * Any malformed structure degrades the whole file to an `error` entry;
//!   the adapter never panics.

#![deny(unsafe_code)]

use std::collections::HashMap;

use halmi_core::adapter::{LanguageAdapter, SourceAnalysis};
use halmi_core::report::{RawFunction, count_f64, count_u32};

/// The flat-script adapter.
#[derive(Debug, Clone, Copy, Default)]
pub struct FlatAdapter;

const OPERATORS: [&str; 8] = ["+", "-", "*", "/", "%", "<", ">", "="];

impl LanguageAdapter for FlatAdapter {
    fn language(&self) -> &'static str {
        "flat-script"
    }

    fn analyze(&self, path: &str, src: &str) -> SourceAnalysis {
        match parse_flat(src) {
            Ok(mut raws) => {
                for raw in &mut raws {
                    raw.path = path.to_string();
                    raw.language = halmi_core::report::Language::Other;
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

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Tokenize a body line into word and symbol tokens.
fn tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in line.chars() {
        if c.is_alphanumeric() || c == '_' {
            cur.push(c);
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            if !c.is_whitespace() {
                out.push(c.to_string());
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn parse_flat(src: &str) -> Result<Vec<RawFunction>, String> {
    let lines: Vec<&str> = src.lines().collect();
    let mut raws = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        let base = indent_of(lines[i]);
        let name = trimmed
            .strip_prefix("def ")
            .and_then(|r| r.strip_suffix(':'))
            .and_then(|r| r.split_whitespace().next())
            .ok_or_else(|| format!("line {}: expected `def NAME:`", i + 1))?;
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(format!("line {}: malformed function name `{name}`", i + 1));
        }
        let start = i + 1;
        let mut body: Vec<&str> = Vec::new();
        let mut j = i + 1;
        while j < lines.len() && (indent_of(lines[j]) > base || lines[j].trim().is_empty()) {
            body.push(lines[j]);
            j += 1;
        }

        while body.last().is_some_and(|l| l.trim().is_empty()) {
            body.pop();
            j -= 1;
        }
        let end = j;

        let mut ops: HashMap<String, usize> = HashMap::new();
        let mut operands: HashMap<String, usize> = HashMap::new();
        let mut cc = 1u32;
        let mut comment_lines = 0u32;
        let mut loc = 0u32;
        for line in &body {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            if t.starts_with('#') {
                comment_lines += 1;
                continue;
            }
            loc += 1;
            for tok in tokens(t) {
                if OPERATORS.contains(&tok.as_str()) {
                    *ops.entry(tok).or_insert(0) += 1;
                } else if tok == "if" {
                    cc += 1;
                } else if tok != "def" {
                    *operands.entry(tok).or_insert(0) += 1;
                }
            }
        }
        let span_lines = count_f64(end - start + 1);
        raws.push(RawFunction {
            path: String::new(),
            language: halmi_core::report::Language::Other,
            name: name.to_string(),
            start_line: count_u32(start),
            end_line: count_u32(end),
            counts: halmi_core::HalsteadCounts {
                eta1: ops.len(),
                eta2: operands.len(),
                n1: ops.values().sum(),
                n2: operands.values().sum(),
                io_params: 0,
            },
            cyclomatic: cc,
            loc,
            nesting: 0,
            comment_density: if span_lines > 0.0 {
                Some(f64::from(comment_lines) / span_lines)
            } else {
                None
            },
            method_chain: 0,
        });
        i = j;
    }
    Ok(raws)
}
