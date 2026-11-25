//! All Halstead software-science metrics and Maintainability Index variants.
//!
//! Every formula here is derived from the raw counts an adapter collects in a
//! single token or AST walk: operator/operand counts, cyclomatic complexity,
//! code-line count, and optional comment density. See docs/internal/halmi-formulas.md.

/// Stroud number: Halstead's estimated psychological discriminations per second.
pub const STROUD_NUMBER: f64 = 18.0;

/// Volume divisor for the delivered-bugs estimate.
pub const BUGS_VOLUME_DIVISOR: f64 = 3000.0;

/// Raw operator/operand counts collected by a language adapter.
///
/// * `eta1` / `eta2`: number of *distinct* operators / operands.
/// * `n1` / `n2`: total occurrences of operators / operands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HalsteadCounts {
    pub eta1: usize,
    pub eta2: usize,
    pub n1: usize,
    pub n2: usize,
    /// eta2*: unique input/output parameters (0 when the adapter cannot
    /// see signatures; drives the potential-volume V* metric).
    pub io_params: usize,
}

/// The full Halstead suite, including fringe metrics.
///
/// All fields are `0.0` when [`HalsteadCounts::compute`] decides the counts
/// are not meaningful (empty or operand-free code).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HalsteadMetrics {
    /// eta = eta1 + eta2
    pub vocabulary: f64,
    /// N = n1 + n2
    pub length: f64,
    /// V = N * log2(eta)
    pub volume: f64,
    /// D = (eta1 / 2) * (n2 / eta2)
    pub difficulty: f64,
    /// E = D * V
    pub effort: f64,
    /// L = 1 / D (0.0 when D is 0)
    pub level: f64,
    /// T = E / 18 seconds
    pub time_seconds: f64,
    /// B = V / 3000
    pub estimated_bugs: f64,
    /// `N_hat` = eta1*log2(eta1) + eta2*log2(eta2)
    pub estimated_length: f64,
    /// `N_hat` / N
    pub purity_ratio: f64,
    /// I = V / D
    pub intelligence: f64,
    /// lambda = L^2 * V
    pub language_level: f64,
    /// Alt length (factorial form): log2(eta1!) + log2(eta2!)
    pub alt_length_factorial: f64,
    /// Alt length (cross form): eta1*log2(eta2) + eta2*log2(eta1)
    pub alt_length_cross: f64,
    /// Alt length (root form): eta1*sqrt(eta1) + eta2*sqrt(eta2)
    pub alt_length_root: f64,
    /// V* = (2 + eta2*) * log2(2 + eta2*): the potential minimum volume
    /// for the same unique i/o parameters. 0.0 when not meaningful.
    pub potential_volume: f64,
    /// False for tiny/degenerate counts; consumers should skip ranking these.
    pub meaningful: bool,
}

/// A per-file line number or count as the `u32` the report schema uses.
///
/// Values come from a single source file, so they sit far below `u32::MAX`;
/// should that ever change the conversion saturates instead of truncating.
#[must_use]
pub fn count_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A per-file count as `f64` for the density ratios.
///
/// `u32` to `f64` is lossless, so every value [`count_u32`] can produce is
/// represented exactly and no ratio rounds differently from a direct cast.
#[must_use]
pub fn count_f64(n: usize) -> f64 {
    f64::from(count_u32(n))
}

/// x * log2(x), defined as 0 for x <= 1 (guards log of 0 and 1).
fn xlog2x(x: f64) -> f64 {
    if x > 1.0 { x * x.log2() } else { 0.0 }
}

/// log2(n!) by summation (exact for our small vocabularies, no overflow).
fn log2_factorial(n: usize) -> f64 {
    let mut acc = 0.0;
    for i in 2..=n {
        acc += count_f64(i).log2();
    }
    acc
}

impl HalsteadCounts {
    /// Compute the full Halstead suite from raw counts.
    ///
    /// Edge cases: division by zero and log of <= 1 are guarded; the result is
    /// flagged `meaningful == false` when there are no operators, no operands,
    /// or vocabulary <= 1 (treat as non-meaningful per halmi-formulas.md).
    #[must_use]
    #[allow(
        clippy::suboptimal_flops,
        reason = "the published formulas are plain a*b + c sequences; fused multiply-add rounds differently and would break bit-exact agreement with the reference values and recorded baselines"
    )]
    pub fn compute(&self) -> HalsteadMetrics {
        let (eta1, eta2, n1, n2) = (
            count_f64(self.eta1),
            count_f64(self.eta2),
            count_f64(self.n1),
            count_f64(self.n2),
        );
        let vocabulary = eta1 + eta2;
        let length = n1 + n2;
        let meaningful = self.n1 > 0 && self.n2 > 0 && vocabulary > 1.0;

        if !meaningful {
            return HalsteadMetrics {
                vocabulary,
                length,
                volume: 0.0,
                difficulty: 0.0,
                effort: 0.0,
                level: 0.0,
                time_seconds: 0.0,
                estimated_bugs: 0.0,
                estimated_length: 0.0,
                purity_ratio: 0.0,
                intelligence: 0.0,
                language_level: 0.0,
                alt_length_factorial: 0.0,
                alt_length_cross: 0.0,
                alt_length_root: 0.0,
                potential_volume: 0.0,
                meaningful: false,
            };
        }

        let volume = if vocabulary > 1.0 {
            length * vocabulary.log2()
        } else {
            0.0
        };
        let difficulty = (eta1 / 2.0) * (n2 / eta2);
        let effort = difficulty * volume;
        let level = if difficulty > 0.0 {
            1.0 / difficulty
        } else {
            0.0
        };
        let time_seconds = effort / STROUD_NUMBER;
        let estimated_bugs = volume / BUGS_VOLUME_DIVISOR;
        let estimated_length = xlog2x(eta1) + xlog2x(eta2);
        let purity_ratio = if length > 0.0 {
            estimated_length / length
        } else {
            0.0
        };
        let intelligence = if difficulty > 0.0 {
            volume / difficulty
        } else {
            0.0
        };
        let language_level = level * level * volume;
        HalsteadMetrics {
            vocabulary,
            length,
            volume,
            difficulty,
            effort,
            level,
            time_seconds,
            estimated_bugs,
            estimated_length,
            purity_ratio,
            intelligence,
            language_level,
            alt_length_factorial: log2_factorial(self.eta1) + log2_factorial(self.eta2),
            alt_length_cross: eta1 * xlog2x(eta2) / eta2 + eta2 * xlog2x(eta1) / eta1,
            alt_length_root: eta1 * eta1.sqrt() + eta2 * eta2.sqrt(),
            potential_volume: xlog2x(2.0 + count_f64(self.io_params)),
            meaningful: true,
        }
    }
}

/// Maintainability band on the 0-100 `mi_vs` scale.
///
/// Per docs/internal/halmi-formulas.md ("common banding"): High >= 85,
/// Moderate 65-85, Low < 65. Absolute bands are a coarse signal; relative
/// ranking within a codebase is preferred (see the formulas doc note).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MiBand {
    Low,
    Moderate,
    High,
}

impl MiBand {
    #[must_use]
    pub fn of(mi_vs: f64) -> Self {
        if mi_vs >= 85.0 {
            Self::High
        } else if mi_vs >= 65.0 {
            Self::Moderate
        } else {
            Self::Low
        }
    }

    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Moderate => "moderate",
            Self::Low => "low",
        }
    }
}

/// Inputs for the Maintainability Index variants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MiInputs {
    pub volume: f64,
    pub cyclomatic: u32,
    /// Code-line count; logical or physical flavor is defined by the adapter.
    pub loc: u32,
    /// Comment density: fraction of comment lines, 0..=1. `None` when the
    /// adapter does not count comments.
    pub comment_density: Option<f64>,
}

/// All Maintainability Index variants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MiMetrics {
    /// Original Coleman/Oman 1992 form, unbounded.
    pub original: f64,
    /// Microsoft Visual Studio form: max(0, original * 100 / 171).
    pub vs: f64,
    /// SEI variant (original + comment term) when comment density was given.
    pub sei: Option<f64>,
    /// False when the function is tiny enough that MI is defined as 100.
    pub meaningful: bool,
}

/// Compute the MI variants from raw inputs plus the Halstead meaningfulness.
///
/// Per halmi-formulas.md: tiny functions (non-meaningful Halstead or LOC = 0)
/// return original MI and MI-VS = 100; SEI is 100 when comment density is
/// provided and `None` otherwise. `ln(V)` and `ln(LOC)` terms are treated as 0
/// when V <= 1 or LOC <= 1.
#[must_use]
#[allow(
    clippy::suboptimal_flops,
    reason = "the published formulas are plain a*b + c sequences; fused multiply-add rounds differently and would break bit-exact agreement with the reference values and recorded baselines"
)]
pub fn compute_mi(inputs: &MiInputs, halstead_meaningful: bool) -> MiMetrics {
    let trivial = !halstead_meaningful || inputs.loc == 0;
    if trivial {
        return MiMetrics {
            original: 100.0,
            vs: 100.0,
            sei: inputs.comment_density.map(|_| 100.0),
            meaningful: false,
        };
    }
    let ln_volume = if inputs.volume > 1.0 {
        inputs.volume.ln()
    } else {
        0.0
    };
    let ln_loc = if inputs.loc > 1 {
        f64::from(inputs.loc).ln()
    } else {
        0.0
    };
    let original = 171.0 - 5.2 * ln_volume - 0.23 * f64::from(inputs.cyclomatic) - 16.2 * ln_loc;
    let vs = (original * 100.0 / 171.0).max(0.0);
    let sei = inputs.comment_density.map(|cm| {
        let cm = cm.clamp(0.0, 1.0);
        original + 50.0 * (2.4 * cm).sqrt().sin()
    });
    MiMetrics {
        original,
        vs,
        sei,
        meaningful: true,
    }
}
