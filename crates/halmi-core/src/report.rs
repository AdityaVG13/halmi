//! The unified `FunctionReport` type.
//!
//! One object per function carrying the full Halstead suite, all MI variants
//! plus band, and the joint agent signal. Serialization is dependency-free (see [`crate::json`]); this
//! module owns the field list, order, and optionality of the wire format.

use crate::json::{Json, write_f64, write_string};
use crate::metrics::{HalsteadCounts, MiBand, MiInputs, compute_mi};
pub use crate::metrics::{count_f64, count_u32};

/// Source language of a report. `Other` covers adapters without a
/// dedicated variant (e.g. the flat-script reference adapter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    Python,
    Go,
    Swift,
    Zig,
    C,
    Cpp,
    Java,
    Cs,
    Kotlin,
    Php,
    Ruby,
    Lua,
    JavaScript,
    TypeScript,
    Other,
}

impl Language {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Go => "go",
            Self::Swift => "swift",
            Self::Zig => "zig",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Java => "java",
            Self::Cs => "csharp",
            Self::Kotlin => "kotlin",
            Self::Php => "php",
            Self::Ruby => "ruby",
            Self::Lua => "lua",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Other => "other",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "rust" => Self::Rust,
            "python" => Self::Python,
            "go" => Self::Go,
            "swift" => Self::Swift,
            "zig" => Self::Zig,
            "c" => Self::C,
            "cpp" => Self::Cpp,
            "java" => Self::Java,
            "csharp" => Self::Cs,
            "kotlin" => Self::Kotlin,
            "php" => Self::Php,
            "ruby" => Self::Ruby,
            "lua" => Self::Lua,
            "javascript" => Self::JavaScript,
            "typescript" => Self::TypeScript,
            "other" => Self::Other,
            _ => return None,
        })
    }
}

/// Version of the JSON schema emitted by halmi.
/// v2: adds `mi_band` (additive; older baselines still parse).
pub const SCHEMA_VERSION: u32 = 2;

/// Per-function delta versus a baseline run (new value minus baseline value).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeltaInfo {
    pub priority: f64,
    pub mi_vs: f64,
    pub effort: f64,
}

/// Classification of a function against its baseline. `New` never carries a
/// [`DeltaInfo`]; every other status does (an invariant [`DeltaStatus`]
/// makes expressible instead of stringly).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaStatus {
    New,
    Worsened,
    Improved,
    Unchanged,
}

impl DeltaStatus {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Worsened => "worsened",
            Self::Improved => "improved",
            Self::Unchanged => "unchanged",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "new" => Self::New,
            "worsened" => Self::Worsened,
            "improved" => Self::Improved,
            "unchanged" => Self::Unchanged,
            _ => return None,
        })
    }
}

/// A function's full metric report. This is the stable, documented JSON unit
/// agents consume; see README.md for the field-by-field schema.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionReport {
    pub path: String,
    pub name: String,
    /// Source language of this function.
    pub language: Language,
    pub start_line: u32,
    pub end_line: u32,

    pub difficulty: f64,
    pub effort: f64,
    pub volume: f64,
    pub estimated_bugs: f64,
    pub purity_ratio: f64,
    /// V*: potential minimum volume for the same unique i/o parameters.
    pub potential_volume: f64,

    /// Microsoft Visual Studio form, clamped to 0..=100.
    pub mi_vs: f64,
    /// SEI variant (with comment term); `None` when comment density is unknown.
    pub mi_sei: Option<f64>,
    /// Original Coleman/Oman form, unbounded.
    pub mi_original: f64,
    /// Band of `mi_vs` (High / Moderate / Low).
    pub mi_band: MiBand,

    pub cyclomatic: u32,
    /// Code-line count; logical or physical flavor is defined by the adapter.
    pub loc: u32,
    /// Max control-flow nesting depth inside the body (body itself = 0).
    pub nesting: u32,
    /// Longest method-call chain on one expression spine (idiom signal;
    /// adapters that cannot see chains report 0).
    pub method_chain: u32,

    /// Higher = fix first. 0.0 for trivial functions.
    pub priority: f64,
    /// Short human/agent-readable explanation for the priority.
    pub reason: String,

    pub delta: Option<DeltaInfo>,
    pub delta_status: Option<DeltaStatus>,
}

impl FunctionReport {
    /// Build a report with `priority = 0` and an empty reason; call
    /// [`crate::ranking::finalize`] to rank and fill in the agent signal.
    #[must_use]
    pub fn from_raw(raw: &RawFunction) -> Self {
        let mut r = Self {
            path: raw.path.clone(),
            name: raw.name.clone(),
            language: raw.language,
            start_line: raw.start_line,
            end_line: raw.end_line,
            difficulty: 0.0,
            effort: 0.0,
            volume: 0.0,
            estimated_bugs: 0.0,
            purity_ratio: 0.0,
            potential_volume: 0.0,
            mi_vs: 0.0,
            mi_sei: None,
            mi_original: 0.0,
            mi_band: crate::metrics::MiBand::High,
            cyclomatic: raw.cyclomatic,
            loc: raw.loc,
            nesting: raw.nesting,
            method_chain: raw.method_chain,
            priority: 0.0,
            reason: String::new(),
            delta: None,
            delta_status: None,
        };
        r.complete_from_raw(raw);
        r
    }

    /// Consuming variant: moves `path` and `name` without cloning.
    #[must_use]
    pub fn from_raw_consuming(mut raw: RawFunction) -> Self {
        let mut r = Self {
            path: std::mem::take(&mut raw.path),
            name: std::mem::take(&mut raw.name),
            language: raw.language,
            start_line: raw.start_line,
            end_line: raw.end_line,
            difficulty: 0.0,
            effort: 0.0,
            volume: 0.0,
            estimated_bugs: 0.0,
            purity_ratio: 0.0,
            potential_volume: 0.0,
            mi_vs: 0.0,
            mi_sei: None,
            mi_original: 0.0,
            mi_band: crate::metrics::MiBand::High,
            cyclomatic: raw.cyclomatic,
            loc: raw.loc,
            nesting: raw.nesting,
            method_chain: raw.method_chain,
            priority: 0.0,
            reason: String::new(),
            delta: None,
            delta_status: None,
        };
        r.complete_from_raw(&raw);
        r
    }

    fn complete_from_raw(&mut self, raw: &RawFunction) {
        let h = raw.counts.compute();
        let mi = compute_mi(
            &MiInputs {
                volume: h.volume,
                cyclomatic: raw.cyclomatic,
                loc: raw.loc,
                comment_density: raw.comment_density,
            },
            h.meaningful,
        );
        self.difficulty = round3(h.difficulty);
        self.effort = round3(h.effort);
        self.volume = round3(h.volume);
        self.estimated_bugs = round3(h.estimated_bugs);
        self.purity_ratio = round3(h.purity_ratio);
        self.potential_volume = round3(h.potential_volume);
        self.mi_vs = round3(mi.vs);
        self.mi_sei = mi.sei.map(round3);
        self.mi_original = round3(mi.original);
        self.mi_band = MiBand::of(round3(mi.vs));
    }

    /// Serialize to a single-line JSON object (the documented field order).
    #[must_use]
    pub fn to_json_string(&self) -> String {
        let mut out = String::new();
        write_report_json(&mut out, self);
        out
    }

    /// Rebuild from a parsed JSON object. `mi_band` is optional (schema v1
    /// baselines) and is recomputed from `mi_vs` when absent.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the value is not an object or a required field is
    /// missing or has the wrong JSON type.
    pub fn from_json(value: &Json) -> Result<Self, String> {
        if value.as_object().is_none() {
            return Err("report is not a JSON object".into());
        }
        let obj_json = value;
        let field = |name: &str| -> Result<&Json, String> {
            obj_json
                .get(name)
                .ok_or_else(|| format!("missing field `{name}`"))
        };
        let u32_of = |name: &str| -> Result<u32, String> {
            field(name)?
                .as_u32()
                .ok_or_else(|| format!("field `{name}` is not an unsigned integer"))
        };
        let f64_of = |name: &str| -> Result<f64, String> {
            field(name)?
                .as_f64()
                .ok_or_else(|| format!("field `{name}` is not a number"))
        };
        let str_of = |name: &str| -> Result<String, String> {
            field(name)?
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("field `{name}` is not a string"))
        };
        let mi_vs = f64_of("mi_vs")?;
        let mi_sei = match obj_json.get("mi_sei") {
            None | Some(Json::Null) => None,
            Some(v) => Some(v.as_f64().ok_or("field `mi_sei` is not a number")?),
        };
        let delta = match obj_json.get("delta") {
            None | Some(Json::Null) => None,
            Some(v) => {
                let d = |k: &str| -> Result<f64, String> {
                    v.get(k)
                        .and_then(Json::as_f64)
                        .ok_or_else(|| format!("delta.{k} is not a number"))
                };
                Some(DeltaInfo {
                    priority: d("priority")?,
                    mi_vs: d("mi_vs")?,
                    effort: d("effort")?,
                })
            }
        };
        let delta_status = match obj_json.get("delta_status") {
            None | Some(Json::Null) => None,
            Some(Json::Str(s)) => {
                Some(DeltaStatus::parse(s).ok_or_else(|| format!("unknown delta_status `{s}`"))?)
            }
            Some(_) => return Err("field `delta_status` is not a string".into()),
        };
        Ok(Self {
            path: str_of("path")?,
            name: str_of("name")?,
            language: match obj_json.get("language") {
                Some(Json::Str(s)) => Language::parse(s).unwrap_or(Language::Other),
                _ => Language::Other,
            },
            start_line: u32_of("start_line")?,
            end_line: u32_of("end_line")?,
            difficulty: f64_of("difficulty")?,
            effort: f64_of("effort")?,
            volume: f64_of("volume")?,
            estimated_bugs: f64_of("estimated_bugs")?,
            purity_ratio: f64_of("purity_ratio")?,
            potential_volume: match obj_json.get("potential_volume") {
                Some(v) => v
                    .as_f64()
                    .ok_or("field `potential_volume` is not a number")?,
                None => 0.0,
            },
            mi_vs,
            mi_sei,
            mi_original: f64_of("mi_original")?,
            mi_band: match obj_json.get("mi_band") {
                Some(Json::Str(s)) if s == "high" => MiBand::High,
                Some(Json::Str(s)) if s == "moderate" => MiBand::Moderate,
                Some(Json::Str(s)) if s == "low" => MiBand::Low,
                None => MiBand::of(mi_vs),
                _ => return Err("field `mi_band` is not a band string".into()),
            },
            cyclomatic: u32_of("cyclomatic")?,
            loc: u32_of("loc")?,
            nesting: u32_of("nesting")?,

            method_chain: match obj_json.get("method_chain") {
                Some(v) => v
                    .as_u32()
                    .ok_or("field `method_chain` is not an unsigned integer")?,
                None => 0,
            },
            priority: f64_of("priority")?,
            reason: str_of("reason")?,
            delta,
            delta_status,
        })
    }
}

/// Append one report as a JSON object to `out`, in the documented field
/// order. The CLI envelope calls this for every result so the field list
/// lives in exactly one place.
pub fn write_report_json(out: &mut String, r: &FunctionReport) {
    out.push_str("{\"path\":");
    write_string(out, &r.path);
    out.push_str(",\"name\":");
    write_string(out, &r.name);
    out.push_str(",\"language\":\"");
    out.push_str(r.language.as_str());
    out.push('"');
    out.push_str(",\"start_line\":");
    let _ = std::fmt::Write::write_fmt(out, format_args!("{}", r.start_line));
    out.push_str(",\"end_line\":");
    let _ = std::fmt::Write::write_fmt(out, format_args!("{}", r.end_line));
    out.push_str(",\"difficulty\":");
    write_f64(out, r.difficulty);
    out.push_str(",\"effort\":");
    write_f64(out, r.effort);
    out.push_str(",\"volume\":");
    write_f64(out, r.volume);
    out.push_str(",\"estimated_bugs\":");
    write_f64(out, r.estimated_bugs);
    out.push_str(",\"purity_ratio\":");
    write_f64(out, r.purity_ratio);
    out.push_str(",\"potential_volume\":");
    write_f64(out, r.potential_volume);
    out.push_str(",\"mi_vs\":");
    write_f64(out, r.mi_vs);
    out.push_str(",\"mi_sei\":");
    match r.mi_sei {
        Some(v) => write_f64(out, v),
        None => out.push_str("null"),
    }
    out.push_str(",\"mi_original\":");
    write_f64(out, r.mi_original);
    out.push_str(",\"mi_band\":\"");
    out.push_str(r.mi_band.as_str());
    out.push_str("\",\"cyclomatic\":");
    let _ = std::fmt::Write::write_fmt(out, format_args!("{}", r.cyclomatic));
    out.push_str(",\"loc\":");
    let _ = std::fmt::Write::write_fmt(out, format_args!("{}", r.loc));
    out.push_str(",\"nesting\":");
    let _ = std::fmt::Write::write_fmt(out, format_args!("{}", r.nesting));
    out.push_str(",\"method_chain\":");
    let _ = std::fmt::Write::write_fmt(out, format_args!("{}", r.method_chain));
    out.push_str(",\"priority\":");
    write_f64(out, r.priority);
    out.push_str(",\"reason\":");
    write_string(out, &r.reason);
    if let Some(d) = r.delta {
        out.push_str(",\"delta\":{\"priority\":");
        write_f64(out, d.priority);
        out.push_str(",\"mi_vs\":");
        write_f64(out, d.mi_vs);
        out.push_str(",\"effort\":");
        write_f64(out, d.effort);
        out.push('}');
    }
    if let Some(s) = r.delta_status {
        out.push_str(",\"delta_status\":\"");
        out.push_str(s.as_str());
        out.push('"');
    }
    out.push('}');
}

/// Parse a baseline: either a full halmi run (`{"results": [...]}`) or a
/// bare JSON array of reports. Unknown fields are ignored.
///
/// # Errors
///
/// Returns `Err` for malformed JSON, a top-level value of another shape, or
/// a report that fails [`FunctionReport::from_json`].
pub fn parse_reports(text: &str) -> Result<Vec<FunctionReport>, String> {
    let v = crate::json::parse(text)?;
    match &v {
        Json::Arr(items) => items.iter().map(FunctionReport::from_json).collect(),
        Json::Obj(_) => match v.get("results") {
            Some(Json::Arr(items)) => items.iter().map(FunctionReport::from_json).collect(),
            _ => Err("baseline is not valid halmi JSON (no results array)".into()),
        },
        _ => Err("baseline is not valid halmi JSON".into()),
    }
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Everything a language adapter collects about one function in a single
/// AST walk. Convert with [`FunctionReport::from_raw`].
#[derive(Debug, Clone, PartialEq)]
pub struct RawFunction {
    /// File path as reported to the user (usually relative to the scan root).
    pub path: String,
    /// Source language, set by the adapter.
    pub language: Language,
    /// Function name; methods are qualified like `Type::method`.
    pub name: String,
    /// 1-based inclusive line span of the signature start through body end.
    pub start_line: u32,
    pub end_line: u32,
    /// Halstead operator/operand counts (the `fn` keyword counts as one
    /// operator occurrence in the Rust adapter).
    pub counts: HalsteadCounts,
    /// Cyclomatic complexity, starting at 1.
    pub cyclomatic: u32,
    /// Code-line count; logical or physical flavor is defined by the adapter.
    pub loc: u32,
    /// Max control-flow nesting depth (function body = 0).
    pub nesting: u32,
    /// Fraction of comment lines inside the span, if the adapter counts them.
    pub comment_density: Option<f64>,
    /// Longest chain of consecutive method-style calls on one expression
    /// spine (idiom signal). Adapters that cannot see chains report 0.
    pub method_chain: u32,
}
