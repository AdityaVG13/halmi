//! Shared fixtures for the halmi-rs adapter tests.

use halmi_core::report::FunctionReport;
use halmi_rs::analyze_source;

/// Analyze inline Rust source under the path "test.rs".
pub fn analyze(src: &str) -> Vec<FunctionReport> {
    analyze_source("test.rs", src).functions
}

/// Fetch one function's report by name, panicking with the full report list
/// when it is missing. (Not every test target in this crate uses it.)
#[allow(dead_code)]
pub fn find<'a>(reports: &'a [FunctionReport], name: &str) -> &'a FunctionReport {
    reports
        .iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("function {name} not found in {reports:?}"))
}
