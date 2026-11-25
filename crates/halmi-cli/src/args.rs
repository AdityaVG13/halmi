//! Command-line surface: flag types, parsing, and validation.

use std::path::PathBuf;

use halmi_core::gates::Gates;
use halmi_core::metrics::MiBand;

use halmi_core::report::FunctionReport;

use crate::USAGE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Table,
    Sarif,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Priority,
    Effort,
    Difficulty,
    Mi,
    Cc,
    Loc,
}

impl SortKey {
    pub(crate) fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "priority" => Self::Priority,
            "effort" => Self::Effort,
            "difficulty" => Self::Difficulty,
            "mi" => Self::Mi,
            "cc" => Self::Cc,
            "loc" => Self::Loc,
            _ => return None,
        })
    }

    pub(crate) fn value(self, r: &FunctionReport) -> f64 {
        match self {
            Self::Priority => r.priority,
            Self::Effort => r.effort,
            Self::Difficulty => r.difficulty,
            Self::Mi => r.mi_vs,
            Self::Cc => f64::from(r.cyclomatic),
            Self::Loc => f64::from(r.loc),
        }
    }

    pub(crate) const fn ascending(self) -> bool {
        matches!(self, Self::Mi)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone)]
pub struct Args {
    pub(crate) paths: Vec<String>,
    pub(crate) format: Format,
    pub(crate) top: usize,
    pub(crate) sort: SortKey,
    pub(crate) min_priority: Option<f64>,
    pub(crate) min_effort: Option<f64>,
    pub(crate) min_difficulty: Option<f64>,
    pub(crate) min_volume: Option<f64>,
    pub(crate) min_loc: Option<f64>,
    pub(crate) by_file: bool,
    pub(crate) color: Color,
    pub(crate) baseline: Option<PathBuf>,
    pub(crate) changed_only: bool,
    pub(crate) excludes: Vec<String>,
    pub(crate) gates: Gates,
}

/// Which flags the user passed explicitly (config-file values only fill
/// the gaps; CLI always wins). Each flag is an independent "was this set
/// on the command line" bit, not a state to encode as an enum.
#[derive(Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct Explicit {
    pub(crate) top: bool,
    pub(crate) sort: bool,
    pub(crate) format: bool,
    pub(crate) color: bool,
    pub(crate) by_file: bool,
    pub(crate) exclude: bool,
}

#[expect(
    clippy::large_enum_variant,
    reason = "parsed exactly once per process; boxing the large variant adds an allocation for nothing"
)]
pub enum Parsed {
    Args(Args, Explicit, Option<PathBuf>),
    Help,
    Version,
}

pub fn parse_finite(raw: &str, flag: &str) -> Result<f64, String> {
    let v: f64 = raw
        .parse()
        .map_err(|_| format!("{flag} expects a number"))?;
    if !v.is_finite() {
        return Err(format!("{flag} expects a finite number (got {raw})"));
    }
    Ok(v)
}

/// One argv item: a flag name and, for the `--flag=value` form, its inline value.
struct Flag {
    name: String,
    inline_value: Option<String>,
}

impl Flag {
    fn parse(arg: &str) -> Self {
        if !arg.starts_with('-') {
            return Self {
                name: arg.to_string(),
                inline_value: None,
            };
        }
        match arg.split_once('=') {
            Some((name, value)) => Self {
                name: name.to_string(),
                inline_value: Some(value.to_string()),
            },
            None => Self {
                name: arg.to_string(),
                inline_value: None,
            },
        }
    }

    /// The flag's value: the inline one, else the next argv item.
    fn value(&self, it: &mut dyn Iterator<Item = String>) -> Result<String, String> {
        self.inline_value.as_ref().map_or_else(
            || {
                it.next()
                    .ok_or_else(|| format!("missing value for {}", self.name))
            },
            |v| Ok(v.clone()),
        )
    }
}

pub fn parse_args<I: IntoIterator<Item = String>>(argv: I) -> Result<Parsed, String> {
    let mut opts = Args {
        paths: Vec::new(),
        format: Format::Json,
        top: 20,
        sort: SortKey::Priority,
        min_priority: None,
        min_effort: None,
        min_difficulty: None,
        min_volume: None,
        min_loc: None,
        by_file: false,
        color: Color::Auto,
        baseline: None,
        changed_only: false,
        excludes: Vec::new(),
        gates: Gates::default(),
    };
    let mut paths_only = false;
    let mut explicit = Explicit::default();
    let mut config_path: Option<PathBuf> = None;
    let mut it = argv.into_iter();
    while let Some(arg) = it.next() {
        if paths_only {
            opts.paths.push(arg);
            continue;
        }
        if arg == "--" {
            paths_only = true;
            continue;
        }
        let flag = Flag::parse(&arg);
        match flag.name.as_str() {
            "--help" | "-h" => return Ok(Parsed::Help),
            "--version" | "-V" => return Ok(Parsed::Version),
            "--json" | "--format" | "--top" | "--exclude" | "--sort" | "--by-file" | "--color" => {
                apply_output_flag(&mut opts, &mut explicit, &flag, &mut it)?;
            }
            "--min-priority" | "--min-effort" | "--min-difficulty" | "--min-volume"
            | "--min-loc" => apply_min_filter(&mut opts, &flag.name, &flag.value(&mut it)?)?,
            "--min-mi" | "--min-band" | "--max-difficulty" | "--max-effort" => {
                apply_gate(&mut opts, &flag, &mut it)?;
            }
            "--config" => config_path = Some(PathBuf::from(flag.value(&mut it)?)),
            "--baseline" => opts.baseline = Some(PathBuf::from(flag.value(&mut it)?)),
            "--changed-only" => opts.changed_only = true,
            other if other.starts_with('-') => {
                return Err(format!(
                    "unknown flag `{other}`

{USAGE}"
                ));
            }
            other => opts.paths.push(other.to_string()),
        }
    }
    validate(&mut opts)?;
    Ok(Parsed::Args(opts, explicit, config_path))
}

/// Output-shaping flags: format, row count, sort key, exclusions, coloring.
fn apply_output_flag(
    opts: &mut Args,
    explicit: &mut Explicit,
    flag: &Flag,
    it: &mut dyn Iterator<Item = String>,
) -> Result<(), String> {
    match flag.name.as_str() {
        "--json" => {
            opts.format = Format::Json;
            explicit.format = true;
        }
        "--format" => {
            opts.format = match flag.value(it)?.as_str() {
                "json" => Format::Json,
                "table" => Format::Table,
                "sarif" => Format::Sarif,
                other => return Err(format!("unknown format `{other}` (json|table|sarif)")),
            };
            explicit.format = true;
        }
        "--top" => {
            let raw = flag.value(it)?;
            if raw.starts_with('+') {
                return Err("--top expects a number".to_string());
            }
            opts.top = raw
                .parse()
                .map_err(|_| "--top expects a number".to_string())?;
            explicit.top = true;
        }
        "--exclude" => {
            opts.excludes.push(flag.value(it)?);
            explicit.exclude = true;
        }
        "--sort" => {
            let raw = flag.value(it)?;
            opts.sort = SortKey::parse(&raw).ok_or_else(|| {
                format!("unknown sort `{raw}` (priority|effort|difficulty|mi|cc|loc)")
            })?;
            explicit.sort = true;
        }
        "--by-file" => {
            opts.by_file = true;
            explicit.by_file = true;
        }
        "--color" => {
            opts.color = match flag.value(it)?.as_str() {
                "auto" => Color::Auto,
                "always" => Color::Always,
                "never" => Color::Never,
                other => {
                    return Err(format!("unknown color mode `{other}` (auto|always|never)"));
                }
            };
            explicit.color = true;
        }
        _ => unreachable!("the caller dispatches only output flags here"),
    }
    Ok(())
}

/// A `--min-<metric>` floor: the display filter for one metric column.
fn apply_min_filter(opts: &mut Args, flag: &str, raw: &str) -> Result<(), String> {
    let floor = parse_finite(raw, flag)?;
    match flag {
        "--min-priority" => opts.min_priority = Some(floor),
        "--min-effort" => opts.min_effort = Some(floor),
        "--min-difficulty" => opts.min_difficulty = Some(floor),
        "--min-volume" => opts.min_volume = Some(floor),
        "--min-loc" => opts.min_loc = Some(floor),
        _ => unreachable!("the caller dispatches only --min-* floors here"),
    }
    Ok(())
}

/// The four gate thresholds (`--min-mi`, `--min-band`, `--max-difficulty`,
/// `--max-effort`); they set the exit code, not the visible rows.
fn apply_gate(
    opts: &mut Args,
    flag: &Flag,
    it: &mut dyn Iterator<Item = String>,
) -> Result<(), String> {
    match flag.name.as_str() {
        "--min-mi" => opts.gates.min_mi = Some(parse_finite(&flag.value(it)?, "--min-mi")?),
        "--min-band" => {
            opts.gates.min_band = Some(match flag.value(it)?.as_str() {
                "high" => MiBand::High,
                "moderate" => MiBand::Moderate,
                "low" => MiBand::Low,
                _ => return Err("--min-band expects high, moderate, or low".to_string()),
            });
        }
        "--max-difficulty" => {
            opts.gates.max_difficulty = Some(parse_finite(&flag.value(it)?, "--max-difficulty")?);
        }
        "--max-effort" => {
            opts.gates.max_effort_factor = Some(parse_finite(&flag.value(it)?, "--max-effort")?);
        }
        _ => unreachable!("the caller dispatches only gate flags here"),
    }
    Ok(())
}

/// Cross-flag constraints, checked once the whole argv is consumed. The
/// default path (`.`) is filled in here rather than at the call site.
fn validate(opts: &mut Args) -> Result<(), String> {
    if opts.paths.is_empty() {
        opts.paths.push(".".into());
    }
    if opts.changed_only && opts.baseline.is_none() {
        return Err("--changed-only requires --baseline".to_string());
    }
    if opts.by_file && opts.baseline.is_some() {
        return Err("--by-file does not support --baseline".to_string());
    }
    if opts.by_file && opts.format == Format::Sarif {
        return Err("--by-file supports json and table formats".to_string());
    }
    Ok(())
}
