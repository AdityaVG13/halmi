//! halmi-core: pure Halstead metrics, Maintainability Index variants, the
//! unified [`FunctionReport`] type, joint priority ranking, delta mode, and
//! gate evaluation. No language-specific code lives here.
//!
//! Typical library flow:
//!
//! ```
//! use halmi_core::ranking::{finalize, PriorityConfig};
//! use halmi_core::report::FunctionReport;
//!
//! let mut reports: Vec<FunctionReport> = vec![]; // from an adapter
//! let stats = finalize(&mut reports, &PriorityConfig::default());
//! // reports are now sorted by priority descending with reasons attached
//! ```

#![deny(unsafe_code)]

pub mod adapter;
pub mod delta;
pub mod gates;
pub mod json;
pub mod metrics;
pub mod ranking;
pub mod report;

pub use adapter::{LanguageAdapter, SourceAnalysis};
pub use metrics::{HalsteadCounts, HalsteadMetrics, MiBand, MiInputs, MiMetrics, compute_mi};
pub use ranking::{PriorityConfig, RepoStats, compute_priority, finalize, is_trivial};
pub use report::{
    DeltaInfo, DeltaStatus, FunctionReport, Language, RawFunction, SCHEMA_VERSION, parse_reports,
    write_report_json,
};
