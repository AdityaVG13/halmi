//! Minimal TOML-subset reader for halmi.toml (zero-dependency by design).
//!
//! Supported, and nothing else: `#` comments, blank lines, one `[section]`
//! header per block (top-level keys and `[gates]`), and `key = value` with
//! value one of: basic string ("..."), number (finite f64), true/false, or
//! a single-line array of basic strings. Strict on purpose: unknown keys
//! and unsupported syntax are errors, so typos fail loudly instead of
//! silently configuring nothing.

use crate::args::{Args, Color, Explicit, Format, SortKey};
use halmi_core::gates::Gates;
use halmi_core::metrics::MiBand;

/// Effective (already merged) run settings carried by the config file.
#[derive(Debug, Default)]
pub struct Config {
    pub top: Option<usize>,
    pub sort: Option<SortKey>,
    pub format: Option<Format>,
    pub color: Option<Color>,
    pub by_file: Option<bool>,
    pub min_priority: Option<f64>,
    pub min_effort: Option<f64>,
    pub min_difficulty: Option<f64>,
    pub min_volume: Option<f64>,
    pub min_loc: Option<f64>,
    pub exclude: Vec<String>,
    pub gates: Gates,
}

pub fn load(path: &std::path::Path) -> Result<Config, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read config {}: {e}", path.display()))?;
    parse(&text)
}

fn parse(text: &str) -> Result<Config, String> {
    let mut cfg = Config::default();
    let mut section = String::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let lineno = idx + 1;
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.trim().to_string();
            if section != "gates" {
                return Err(format!("config line {lineno}: unknown section [{section}]"));
            }
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("config line {lineno}: expected `key = value`"))?;
        let key = key.trim();
        let value = value.trim();
        if section == "gates" {
            parse_gate(&mut cfg, key, value, lineno)?;
        } else {
            parse_top(&mut cfg, key, value, lineno)?;
        }
    }
    Ok(cfg)
}

fn number(value: &str, key: &str) -> Result<f64, String> {
    let v: f64 = value
        .parse()
        .map_err(|_| format!("config key `{key}`: expected a number"))?;
    if !v.is_finite() {
        return Err(format!("config key `{key}`: expected a finite number"));
    }
    Ok(v)
}

fn string(value: &str, key: &str) -> Result<String, String> {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .map(str::to_string)
        .ok_or_else(|| format!("config key `{key}`: expected a quoted string"))
}

fn string_array(value: &str, key: &str) -> Result<Vec<String>, String> {
    let inner = value
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .ok_or_else(|| format!("config key `{key}`: expected a [\"...\"] array"))?;
    let inner = inner.trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    inner
        .split(',')
        .map(|item| string(item.trim(), key))
        .collect()
}

fn parse_top(cfg: &mut Config, key: &str, value: &str, lineno: usize) -> Result<(), String> {
    let bad = |what: &str| format!("config line {lineno}: {what}");
    match key {
        "top" => {
            let v = number(value, key).map_err(|e| bad(&e))?;
            if v < 0.0 || v.fract() != 0.0 {
                return Err(bad("top must be a non-negative integer"));
            }
            #[expect(
                clippy::cast_sign_loss,
                clippy::cast_possible_truncation,
                reason = "range-checked above; f64 has no TryFrom<usize>"
            )]
            let top = v as usize;
            cfg.top = Some(top);
        }
        "sort" => {
            let raw = string(value, key).map_err(|e| bad(&e))?;
            cfg.sort = Some(SortKey::parse(&raw).ok_or_else(|| bad("unknown sort key"))?);
        }
        "format" => {
            cfg.format = Some(match value {
                "\"json\"" => Format::Json,
                "\"table\"" => Format::Table,
                _ => return Err(bad("format must be \"json\" or \"table\"")),
            });
        }
        "color" => {
            cfg.color = Some(match value {
                "\"auto\"" => Color::Auto,
                "\"always\"" => Color::Always,
                "\"never\"" => Color::Never,
                _ => return Err(bad("color must be \"auto\", \"always\", or \"never\"")),
            });
        }
        "by_file" => {
            cfg.by_file = Some(value == "true");
            if !matches!(value, "true" | "false") {
                return Err(bad("by_file must be true or false"));
            }
        }
        "min_priority" => cfg.min_priority = Some(number(value, key).map_err(|e| bad(&e))?),
        "min_effort" => cfg.min_effort = Some(number(value, key).map_err(|e| bad(&e))?),
        "min_difficulty" => cfg.min_difficulty = Some(number(value, key).map_err(|e| bad(&e))?),
        "min_volume" => cfg.min_volume = Some(number(value, key).map_err(|e| bad(&e))?),
        "min_loc" => cfg.min_loc = Some(number(value, key).map_err(|e| bad(&e))?),
        "exclude" => cfg.exclude = string_array(value, key).map_err(|e| bad(&e))?,
        other => return Err(bad(&format!("unknown key `{other}`"))),
    }
    Ok(())
}

fn parse_gate(cfg: &mut Config, key: &str, value: &str, lineno: usize) -> Result<(), String> {
    let bad = |what: &str| format!("config line {lineno}: {what}");
    match key {
        "min_mi" => cfg.gates.min_mi = Some(number(value, key).map_err(|e| bad(&e))?),
        "max_difficulty" => {
            cfg.gates.max_difficulty = Some(number(value, key).map_err(|e| bad(&e))?);
        }
        "max_effort" => {
            cfg.gates.max_effort_factor = Some(number(value, key).map_err(|e| bad(&e))?);
        }
        "min_band" => {
            cfg.gates.min_band = Some(match value {
                "\"high\"" => MiBand::High,
                "\"moderate\"" => MiBand::Moderate,
                "\"low\"" => MiBand::Low,
                _ => return Err(bad("min_band must be \"high\", \"moderate\", or \"low\"")),
            });
        }
        other => return Err(bad(&format!("unknown [gates] key `{other}`"))),
    }
    Ok(())
}

/// Overlay config-file defaults onto parsed CLI args; explicit flags win.
pub fn apply_config(
    args: &mut Args,
    explicit: &Explicit,
    config_path: Option<std::path::PathBuf>,
) -> Result<(), String> {
    let explicit_path = config_path.is_some();
    let cfg_path = config_path.unwrap_or_else(|| std::path::PathBuf::from("halmi.toml"));
    if !cfg_path.exists() {
        if explicit_path {
            return Err(format!("config file not found: {}", cfg_path.display()));
        }
        return Ok(());
    }
    let cfg = load(&cfg_path)?;
    if !explicit.top
        && let Some(v) = cfg.top
    {
        args.top = v;
    }
    if !explicit.sort
        && let Some(v) = cfg.sort
    {
        args.sort = v;
    }
    if !explicit.format
        && let Some(v) = cfg.format
    {
        args.format = v;
    }
    if !explicit.color
        && let Some(v) = cfg.color
    {
        args.color = v;
    }
    if !explicit.by_file
        && let Some(v) = cfg.by_file
    {
        args.by_file = v;
    }
    if !explicit.exclude && !cfg.exclude.is_empty() {
        args.excludes = cfg.exclude;
    }
    args.min_priority = args.min_priority.or(cfg.min_priority);
    args.min_effort = args.min_effort.or(cfg.min_effort);
    args.min_difficulty = args.min_difficulty.or(cfg.min_difficulty);
    args.min_volume = args.min_volume.or(cfg.min_volume);
    args.min_loc = args.min_loc.or(cfg.min_loc);
    args.gates.min_mi = args.gates.min_mi.or(cfg.gates.min_mi);
    args.gates.max_difficulty = args.gates.max_difficulty.or(cfg.gates.max_difficulty);
    args.gates.max_effort_factor = args.gates.max_effort_factor.or(cfg.gates.max_effort_factor);
    args.gates.min_band = args.gates.min_band.or(cfg.gates.min_band);
    if args.by_file && args.format == Format::Sarif {
        return Err("--by-file supports json and table formats".to_string());
    }
    if args.by_file && args.baseline.is_some() {
        return Err("--by-file does not support --baseline".to_string());
    }
    Ok(())
}
