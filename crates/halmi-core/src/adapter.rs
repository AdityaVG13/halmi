//! The language-neutral adapter port.
//!
//! halmi-core owns everything downstream of raw collection (metrics,
//! ranking, delta, gates); a language adapter implements
//! [`LanguageAdapter`] and turns source text into [`RawFunction`]s. Adapters are pure: source in, reports out; file
//! discovery and IO stay at the caller's edge.

use crate::report::{FunctionReport, RawFunction};

/// Result of analyzing one source file, independent of language.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceAnalysis {
    /// Path as passed in (used verbatim in reports).
    pub path: String,
    /// One entry per function found, in source order. Unranked.
    pub functions: Vec<FunctionReport>,
    /// Parse error message when the file could not be parsed.
    pub error: Option<String>,
}

/// The port a language adapter implements. The contract:
///
/// * Pure: no IO, no clocks, no global state; identical input gives
///   identical output.
/// * Never panics on malformed input: degrade to `functions: []` plus an
///   `error` message so whole-tree scans keep going past bad files.
/// * One pass over the source per function for all raw signals (Halstead
///   counts, cyclomatic, logical LOC, nesting, spans, comment density).
pub trait LanguageAdapter {
    /// Human-facing language name (used in diagnostics).
    fn language(&self) -> &'static str;

    /// Analyze source text under the given path.
    fn analyze(&self, path: &str, src: &str) -> SourceAnalysis;

    /// Convert raw collections into unranked reports (shared pipeline).
    fn reports_from_raw(&self, raws: Vec<RawFunction>) -> Vec<FunctionReport> {
        raws.into_iter()
            .map(FunctionReport::from_raw_consuming)
            .collect()
    }
}
