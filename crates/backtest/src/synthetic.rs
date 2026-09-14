//! The synthetic venue (DATA-005 §9, DA-13).
//!
//! Seven seeded generators produce OHLCV bars for `venue_id = 'synthetic'`. Three
//! of them contain **no exploitable structure at all**; four contain a single
//! planted mechanism whose exact form is known to the grader and to nobody else.
//!
//! This module is the measuring instrument for AGENT-004. Two properties matter
//! more than realism:
//!
//! 1. **The noise generators must really be noise.** They have fat tails, jumps
//!    and volatility clustering — every feature that makes a series *look*
//!    tradeable — and a conditional mean of exactly zero. An agent that reports an
//!    edge here has failed, and so has whatever machinery let the claim through.
//!    `noise_generators_have_no_detectable_drift` is the test that keeps this
//!    honest as the code changes.
//!
//! 2. **The planted generators must be recoverable.** A power curve measured
//!    against a mechanism too faint to find measures the generator, not the agent.
//!    Each planted variant carries a `Truth` descriptor stating the mechanism and
//!    strength, which is what the grader checks a candidate against.
//!
//! Everything is derived from `(generator, params, seed)` through `DetRng`, so a
//! task definition reproduces byte-identically on any machine, forever. That is
//! what makes "fresh synthetic seeds every run" (AGENT-004 §5) safe: the seed is
//! the whole state.

use crate::rng::DetRng;
use crate::store::CollectedBar;
use crate::types::TimeframeExt;
use chrono::{DateTime, Duration, Timelike, Utc};
use domain::payloads::bar::Timeframe;
use serde::{Deserialize, Serialize};
use std::fmt;

/// The seven generators of DATA-005 §9.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Generator {
    /// GARCH(1,1) with Student-t innovations. Volatility clusters; mean is zero.
    GarchT,
    /// Diffusion plus compound jumps. Fat tails; mean is zero.
    MertonJump,
    /// Two-state Markov volatility regime. Persistent vol; mean is zero.
    RegimeSwitch,
    /// A planted AR(1) in returns with coefficient `phi`.
    PlantedAr1,
    /// A planted drift confined to one hour of the day.
    PlantedHourDrift,
    /// A planted positive drift for `n` bars after a volatility breakout.
    PlantedVolBreakout,
    /// A planted drift proportional to a slow-moving observable carry series.
    PlantedCarry,
}

impl Generator {
    pub const ALL: &'static [Generator] = &[
        Generator::GarchT,
        Generator::MertonJump,
        Generator::RegimeSwitch,
        Generator::PlantedAr1,
        Generator::PlantedHourDrift,
        Generator::PlantedVolBreakout,
        Generator::PlantedCarry,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Generator::GarchT => "garch_t",
            Generator::MertonJump => "merton_jump",
            Generator::RegimeSwitch => "regime_switch",
            Generator::PlantedAr1 => "planted_ar1",
            Generator::PlantedHourDrift => "planted_hour_drift",
            Generator::PlantedVolBreakout => "planted_vol_breakout",
            Generator::PlantedCarry => "planted_carry",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Generator::ALL.iter().copied().find(|g| g.as_str() == s)
    }

    /// Whether this generator plants an exploitable mechanism.
    ///
    /// This is the ground truth the `noise` and `planted_edge` suites are scored
    /// against, and it is deliberately a property of the generator rather than
    /// something a task file asserts: a task that could declare its own answer
    /// would eventually declare the wrong one.
    #[must_use]
    pub fn has_edge(self) -> bool {
        matches!(
            self,
            Generator::PlantedAr1
                | Generator::PlantedHourDrift
                | Generator::PlantedVolBreakout
                | Generator::PlantedCarry
        )
    }
}

impl fmt::Display for Generator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Generator parameters, all defaulted so a task file states only what it varies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenParams {
    /// Per-bar volatility of the base return process.
    #[serde(default = "d_sigma")]
    pub sigma: f64,
    /// Student-t degrees of freedom for `garch_t` (integer, >= 3).
    #[serde(default = "d_nu")]
    pub nu: u32,
    /// GARCH persistence terms.
    #[serde(default = "d_alpha")]
    pub alpha: f64,
    #[serde(default = "d_beta")]
    pub beta: f64,
    /// Jump intensity per bar and jump size scale.
    #[serde(default = "d_jump_lambda")]
    pub jump_lambda: f64,
    #[serde(default = "d_jump_scale")]
    pub jump_scale: f64,
    /// Regime-switching transition probability and the high-vol multiplier.
    #[serde(default = "d_p_switch")]
    pub p_switch: f64,
    #[serde(default = "d_vol_ratio")]
    pub vol_ratio: f64,
    /// `planted_ar1` autocorrelation coefficient.
    #[serde(default = "d_phi")]
    pub phi: f64,
    /// `planted_hour_drift`: the UTC hour that drifts, and the per-bar drift.
    #[serde(default = "d_drift_hour")]
    pub drift_hour: u32,
    #[serde(default = "d_drift")]
    pub drift: f64,
    /// `planted_vol_breakout`: the realised-vol multiple that triggers, and how
    /// many bars the drift persists afterwards.
    #[serde(default = "d_breakout_z")]
    pub breakout_z: f64,
    #[serde(default = "d_breakout_bars")]
    pub breakout_bars: usize,
    /// `planted_carry`: how strongly the observable carry maps into drift.
    #[serde(default = "d_carry_beta")]
    pub carry_beta: f64,
    #[serde(default = "d_carry_rho")]
    pub carry_rho: f64,
    /// Starting price.
    #[serde(default = "d_start_price")]
    pub start_price: f64,
    /// Mean per-bar volume, used only to make the series look like a market.
    #[serde(default = "d_volume")]
    pub volume: f64,
}

fn d_sigma() -> f64 {
    0.0015
}
fn d_nu() -> u32 {
    4
}
fn d_alpha() -> f64 {
    0.08
}
fn d_beta() -> f64 {
    0.90
}
fn d_jump_lambda() -> f64 {
    0.01
}
fn d_jump_scale() -> f64 {
    0.02
}
fn d_p_switch() -> f64 {
    0.01
}
fn d_vol_ratio() -> f64 {
    3.0
}
fn d_phi() -> f64 {
    0.05
}
fn d_drift_hour() -> u32 {
    14
}
fn d_drift() -> f64 {
    0.0008
}
fn d_breakout_z() -> f64 {
    2.0
}
fn d_breakout_bars() -> usize {
    6
}
fn d_carry_beta() -> f64 {
    0.0015
}
fn d_carry_rho() -> f64 {
    0.995
}
fn d_start_price() -> f64 {
    100.0
}
fn d_volume() -> f64 {
    1000.0
}

impl Default for GenParams {
    fn default() -> Self {
        serde_json::from_str("{}").expect("all GenParams fields are defaulted")
    }
}

/// What was actually planted. Held by the grader; never shown to an agent token.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Truth {
    /// A stable machine name for the mechanism, e.g. `"ar1_return_autocorrelation"`.
    pub mechanism: &'static str,
    /// Human description of what a correct candidate must exploit.
    pub description: String,
    /// The planted strength, in the mechanism's own units.
    pub strength: f64,
    /// Whether an edge exists at all. `false` for the three noise generators.
    pub exploitable: bool,
}

/// A full synthetic instrument request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticSpec {
    pub generator: Generator,
    #[serde(default)]
    pub params: GenParams,
    pub seed: u64,
    /// Bar timeframe key, e.g. `"1h"`.
    pub timeframe: String,
    /// Number of bars to generate.
    pub length: usize,
    /// First bar's close (`available_time`). Bars run forward from here.
    pub start: DateTime<Utc>,
}

impl SyntheticSpec {
    /// The instrument id, `SYN-<generator>-<seed>` (DATA-005 §9).
    #[must_use]
    pub fn instrument_id(&self) -> String {
        format!(
            "SYN-{}-{}",
            self.generator.as_str().to_uppercase().replace('_', "-"),
            self.seed
        )
    }

    pub fn resolve_timeframe(&self) -> Result<Timeframe, SyntheticError> {
        <Timeframe as TimeframeExt>::from_key(&self.timeframe)
            .ok_or_else(|| SyntheticError::BadTimeframe(self.timeframe.clone()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SyntheticError {
    #[error("unknown timeframe {0:?}")]
    BadTimeframe(String),
    #[error("length must be between 1 and {max}, got {got}")]
    BadLength { got: usize, max: usize },
    #[error("parameter {name} is out of range: {detail}")]
    BadParam { name: &'static str, detail: String },
}

/// Hard ceiling on one request. A generator is cheap; a caller asking for a
/// hundred million bars is a mistake, and finding that out by filling ClickHouse
/// is an expensive way to learn it.
pub const MAX_LENGTH: usize = 2_000_000;

// -- The generators ----------------------------------------------------------

/// A standard normal via Box-Muller. Deterministic given the stream.
fn normal(rng: &mut DetRng) -> f64 {
    // `next_f64` is in [0,1); u1 must be strictly positive for the logarithm.
    let u1 = rng.next_f64().max(f64::MIN_POSITIVE);
    let u2 = rng.next_f64();
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

/// A standard Student-t with `nu` degrees of freedom, scaled to unit variance
/// (for `nu > 2`). The scaling matters: without it, changing `nu` silently
/// changes the volatility of the whole series, and any power curve measured
/// across `nu` values would be comparing two different things.
fn student_t(rng: &mut DetRng, nu: u32) -> f64 {
    let nu = nu.max(3);
    let z = normal(rng);
    let chi2: f64 = (0..nu).map(|_| normal(rng).powi(2)).sum();
    let raw = z / (chi2 / f64::from(nu)).sqrt();
    let nu_f = f64::from(nu);
    raw / (nu_f / (nu_f - 2.0)).sqrt()
}

/// Checks a spec without generating anything.
///
/// Split out from [`returns`] so an HTTP handler can reject a bad request with a
/// 422 on the call that made it, rather than accepting a job the caller has to poll
/// in order to discover it was never going to work.
pub fn validate(spec: &SyntheticSpec) -> Result<(), SyntheticError> {
    if spec.length == 0 || spec.length > MAX_LENGTH {
        return Err(SyntheticError::BadLength {
            got: spec.length,
            max: MAX_LENGTH,
        });
    }
    spec.resolve_timeframe()?;
    let p = &spec.params;
    if p.sigma <= 0.0 {
        return Err(SyntheticError::BadParam {
            name: "sigma",
            detail: format!("must be positive, got {}", p.sigma),
        });
    }
    if p.alpha + p.beta >= 1.0 {
        return Err(SyntheticError::BadParam {
            name: "alpha+beta",
            detail: format!(
                "GARCH must be stationary: alpha + beta < 1, got {}",
                p.alpha + p.beta
            ),
        });
    }
    if p.phi.abs() >= 1.0 {
        return Err(SyntheticError::BadParam {
            name: "phi",
            detail: format!("must satisfy |phi| < 1, got {}", p.phi),
        });
    }
    if p.drift_hour > 23 {
        return Err(SyntheticError::BadParam {
            name: "drift_hour",
            detail: format!("must be a UTC hour in 0..=23, got {}", p.drift_hour),
        });
    }
    if p.carry_rho.abs() >= 1.0 {
        return Err(SyntheticError::BadParam {
            name: "carry_rho",
            detail: format!("must satisfy |rho| < 1, got {}", p.carry_rho),
        });
    }
    if p.start_price <= 0.0 {
        return Err(SyntheticError::BadParam {
            name: "start_price",
            detail: format!("must be positive, got {}", p.start_price),
        });
    }
    Ok(())
}

/// The per-bar log returns for a spec, plus the truth descriptor.
///
/// Returns are produced here and turned into OHLCV by [`generate`]. Keeping the
/// two apart is what lets the noise tests exercise the process itself rather than
/// the bar construction around it.
#[allow(clippy::too_many_lines)]
pub fn returns(spec: &SyntheticSpec) -> Result<(Vec<f64>, Truth), SyntheticError> {
    validate(spec)?;
    let p = &spec.params;

    // Every generator draws from one stream seeded by (generator, seed) so that
    // the same seed under two generators does not produce correlated series.
    let mut rng = DetRng::new(spec.seed ^ generator_salt(spec.generator));
    let n = spec.length;
    let tf = spec.resolve_timeframe()?;
    let period = i64::try_from(tf.seconds()).unwrap_or(3600);

    let (r, truth) = match spec.generator {
        Generator::GarchT => {
            let omega = p.sigma.powi(2) * (1.0 - p.alpha - p.beta);
            let mut h = p.sigma.powi(2);
            let mut prev = 0.0_f64;
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                h = omega + p.alpha * prev.powi(2) + p.beta * h;
                let e = h.sqrt() * student_t(&mut rng, p.nu);
                out.push(e);
                prev = e;
            }
            (
                out,
                Truth {
                    mechanism: "none",
                    description:
                        "GARCH(1,1)-t: volatility clusters, conditional mean is exactly zero".into(),
                    strength: 0.0,
                    exploitable: false,
                },
            )
        }
        Generator::MertonJump => {
            // Compensate the jump so the unconditional mean stays zero. The jump
            // sizes are symmetric, so the compensator is zero here; the constant
            // is kept explicit because a future asymmetric jump would otherwise
            // introduce a real drift and make "no edge" a lie.
            let compensator = 0.0;
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                let diffusion = p.sigma * normal(&mut rng);
                // At the intensities we use, the Poisson count is 0 or 1 in all
                // but a vanishing fraction of bars; a Bernoulli draw is the same
                // process to any precision the suite can measure.
                let jump = if rng.next_f64() < p.jump_lambda {
                    p.jump_scale * normal(&mut rng)
                } else {
                    0.0
                };
                out.push(diffusion + jump - compensator);
            }
            (
                out,
                Truth {
                    mechanism: "none",
                    description: "jump-diffusion: fat tails, conditional mean is zero".into(),
                    strength: 0.0,
                    exploitable: false,
                },
            )
        }
        Generator::RegimeSwitch => {
            let mut high = false;
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                if rng.next_f64() < p.p_switch {
                    high = !high;
                }
                let s = if high { p.sigma * p.vol_ratio } else { p.sigma };
                out.push(s * normal(&mut rng));
            }
            (
                out,
                Truth {
                    mechanism: "none",
                    description:
                        "two-state volatility regime switching; the mean is zero in both states"
                            .into(),
                    strength: 0.0,
                    exploitable: false,
                },
            )
        }
        Generator::PlantedAr1 => {
            let mut prev = 0.0_f64;
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                // Scale the innovation so the unconditional variance stays
                // sigma^2 regardless of phi. Otherwise a larger phi both adds
                // signal and raises volatility, and the power curve is no longer
                // a curve in one variable.
                let innovation = p.sigma * (1.0 - p.phi.powi(2)).sqrt() * normal(&mut rng);
                let r = p.phi * prev + innovation;
                out.push(r);
                prev = r;
            }
            (
                out,
                Truth {
                    mechanism: "ar1_return_autocorrelation",
                    description: format!(
                        "returns follow an AR(1) with phi={}; the lag-1 return predicts the next bar",
                        p.phi
                    ),
                    strength: p.phi,
                    exploitable: true,
                },
            )
        }
        Generator::PlantedHourDrift => {
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let ts = spec.start + Duration::seconds(period * i64::try_from(i).unwrap_or(0));
                let d = if ts.hour() == p.drift_hour {
                    p.drift
                } else {
                    0.0
                };
                out.push(d + p.sigma * normal(&mut rng));
            }
            (
                out,
                Truth {
                    mechanism: "hour_of_day_drift",
                    description: format!(
                        "bars closing in UTC hour {} carry a mean return of {}",
                        p.drift_hour, p.drift
                    ),
                    strength: p.drift,
                    exploitable: true,
                },
            )
        }
        Generator::PlantedVolBreakout => {
            let window = 24usize;
            let mut out: Vec<f64> = Vec::with_capacity(n);
            let mut remaining = 0usize;
            for i in 0..n {
                let d = if remaining > 0 {
                    remaining -= 1;
                    p.drift
                } else {
                    0.0
                };
                let r = d + p.sigma * normal(&mut rng);
                out.push(r);
                // Arm the drift when realised vol over the trailing window
                // exceeds `breakout_z` times the base. The trigger reads only
                // bars up to and including `i`, which is the whole point: a
                // planted edge that needs the future is not an edge.
                if i + 1 >= window && remaining == 0 {
                    let seg = &out[i + 1 - window..=i];
                    let mean = seg.iter().sum::<f64>() / window as f64;
                    let var =
                        seg.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (window as f64 - 1.0);
                    if var.sqrt() > p.breakout_z * p.sigma {
                        remaining = p.breakout_bars;
                    }
                }
            }
            (
                out,
                Truth {
                    mechanism: "vol_breakout_drift",
                    description: format!(
                        "trailing-24-bar realised vol above {}x base is followed by {} bars of {} drift",
                        p.breakout_z, p.breakout_bars, p.drift
                    ),
                    strength: p.drift,
                    exploitable: true,
                },
            )
        }
        Generator::PlantedCarry => {
            // The carry series is an observable the agent can fetch; the drift is
            // a function of its *lagged* value, so acting on it is legitimate.
            let mut carry = 0.0_f64;
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                let lagged = carry;
                out.push(p.carry_beta * lagged + p.sigma * normal(&mut rng));
                carry = p.carry_rho * carry + (1.0 - p.carry_rho.powi(2)).sqrt() * normal(&mut rng);
            }
            (
                out,
                Truth {
                    mechanism: "lagged_carry_drift",
                    description: format!(
                        "drift equals {} times the previous bar's carry observable (AR rho {})",
                        p.carry_beta, p.carry_rho
                    ),
                    strength: p.carry_beta,
                    exploitable: true,
                },
            )
        }
    };

    debug_assert_eq!(r.len(), n);
    Ok((r, truth))
}

/// The observable carry series that accompanies `planted_carry`.
///
/// It is regenerated from the same seed rather than stored, which keeps the
/// instrument a pure function of `(generator, params, seed)` and means an agent
/// reading the carry endpoint and a grader checking the mechanism are looking at
/// the same numbers by construction.
#[must_use]
pub fn carry_series(spec: &SyntheticSpec) -> Vec<f64> {
    if spec.generator != Generator::PlantedCarry {
        return Vec::new();
    }
    let p = &spec.params;
    let mut rng = DetRng::new(spec.seed ^ generator_salt(spec.generator));
    let mut carry = 0.0_f64;
    let mut out = Vec::with_capacity(spec.length);
    for _ in 0..spec.length {
        out.push(carry);
        // Consume the return draw so this stream stays aligned with `returns`.
        let _ = normal(&mut rng);
        carry = p.carry_rho * carry + (1.0 - p.carry_rho.powi(2)).sqrt() * normal(&mut rng);
    }
    out
}

/// A per-generator salt, so seed 7 under `garch_t` and seed 7 under `planted_ar1`
/// are independent series rather than the same noise with different labels.
fn generator_salt(g: Generator) -> u64 {
    // FNV-1a over the generator name: stable across builds and platforms.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in g.as_str().as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Salt for the bar-construction stream, kept separate from the return stream so
/// that changing the wick model cannot change the returns.
const WICK_SALT: u64 = 0x5713_CB40_9E11_0001;

/// Generate bars for a spec.
///
/// The OHLC of each bar is built from its own close-to-close return plus an
/// intrabar excursion. The excursion is drawn *after* the return and applied
/// symmetrically, so the high and low carry no information about the next bar — a
/// synthetic series whose wicks predicted the future would plant an edge in every
/// generator, including the three that are supposed to have none.
pub fn generate(spec: &SyntheticSpec) -> Result<(Vec<CollectedBar>, Truth), SyntheticError> {
    let (rets, truth) = returns(spec)?;
    let tf = spec.resolve_timeframe()?;
    let period = i64::try_from(tf.seconds()).unwrap_or(3600);
    let mut wick_rng = DetRng::new(spec.seed ^ WICK_SALT);
    let mut price = spec.params.start_price;
    let mut bars = Vec::with_capacity(rets.len());

    for (i, r) in rets.iter().enumerate() {
        let open = price;
        let close = (open * r.exp()).max(1e-8);
        let excursion = wick_rng.next_f64() * spec.params.sigma;
        let hi = open.max(close) * (1.0 + excursion);
        let lo = open.min(close) * (1.0 - excursion);
        let available_time = spec.start + Duration::seconds(period * i64::try_from(i).unwrap_or(0));
        let volume = spec.params.volume * (0.5 + wick_rng.next_f64());
        bars.push(CollectedBar {
            available_time,
            // The sequence is the bar's open in epoch seconds, matching what the
            // real collectors write, so the dedup key has the same shape.
            sequence: u64::try_from(available_time.timestamp() - period).unwrap_or(0),
            open: fmt_px(open),
            high: fmt_px(hi),
            low: fmt_px(lo),
            close: fmt_px(close),
            volume: fmt_px(volume),
            trade_count: 1,
        });
        price = close;
    }
    Ok((bars, truth))
}

/// Decimal string with 8 places — inside `Decimal128(10)`, and never a float in
/// the wire format.
fn fmt_px(x: f64) -> String {
    format!("{x:.8}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn spec(g: Generator, seed: u64, length: usize) -> SyntheticSpec {
        SyntheticSpec {
            generator: g,
            params: GenParams::default(),
            seed,
            timeframe: "1h".into(),
            length,
            start: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    fn mean(xs: &[f64]) -> f64 {
        xs.iter().sum::<f64>() / xs.len() as f64
    }

    fn stdev(xs: &[f64]) -> f64 {
        let m = mean(xs);
        (xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (xs.len() as f64 - 1.0)).sqrt()
    }

    /// Lag-1 autocorrelation.
    fn acf1(xs: &[f64]) -> f64 {
        let m = mean(xs);
        let num: f64 = xs.windows(2).map(|w| (w[0] - m) * (w[1] - m)).sum();
        let den: f64 = xs.iter().map(|x| (x - m).powi(2)).sum();
        num / den
    }

    #[test]
    fn the_same_seed_reproduces_the_same_series() {
        for g in Generator::ALL {
            let a = generate(&spec(*g, 12345, 500)).unwrap().0;
            let b = generate(&spec(*g, 12345, 500)).unwrap().0;
            assert_eq!(
                a.iter().map(|x| x.close.clone()).collect::<Vec<_>>(),
                b.iter().map(|x| x.close.clone()).collect::<Vec<_>>(),
                "{g} is not reproducible from its seed"
            );
        }
    }

    #[test]
    fn different_generators_on_one_seed_are_independent() {
        let a = returns(&spec(Generator::GarchT, 7, 2000)).unwrap().0;
        let b = returns(&spec(Generator::MertonJump, 7, 2000)).unwrap().0;
        let (ma, mb) = (mean(&a), mean(&b));
        let cov: f64 = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - ma) * (y - mb))
            .sum::<f64>()
            / a.len() as f64;
        let corr = cov / (stdev(&a) * stdev(&b));
        assert!(
            corr.abs() < 0.1,
            "the same seed under two generators correlates at {corr}"
        );
    }

    /// The property the whole `noise` suite rests on: none of the three noise
    /// generators has a drift a strategy could harvest. The bound is 3 standard
    /// errors of the sample mean, which is the same test an honest agent would
    /// run.
    #[test]
    fn noise_generators_have_no_detectable_drift() {
        let n = 20_000;
        for g in [
            Generator::GarchT,
            Generator::MertonJump,
            Generator::RegimeSwitch,
        ] {
            for seed in [1u64, 99, 4242] {
                let r = returns(&spec(g, seed, n)).unwrap().0;
                let se = stdev(&r) / (n as f64).sqrt();
                let t = mean(&r) / se;
                assert!(
                    t.abs() < 3.0,
                    "{g} seed {seed} has a drift at t={t:.2} — it is not noise"
                );
            }
        }
    }

    /// And no lag-1 autocorrelation either, which is the cheapest edge an agent
    /// would find first.
    #[test]
    fn noise_generators_have_no_return_autocorrelation() {
        let n = 20_000;
        for g in [
            Generator::GarchT,
            Generator::MertonJump,
            Generator::RegimeSwitch,
        ] {
            for seed in [5u64, 777] {
                let r = returns(&spec(g, seed, n)).unwrap().0;
                let a = acf1(&r);
                assert!(
                    a.abs() < 3.0 / (n as f64).sqrt(),
                    "{g} seed {seed} has lag-1 acf {a:.4}"
                );
            }
        }
    }

    /// The mirror image: each planted generator must actually be recoverable, or
    /// a power curve measured against it is measuring the generator.
    #[test]
    fn planted_ar1_is_recoverable_at_every_strength() {
        for phi in [0.02, 0.05, 0.1] {
            let mut s = spec(Generator::PlantedAr1, 31, 40_000);
            s.params.phi = phi;
            let r = returns(&s).unwrap().0;
            let a = acf1(&r);
            assert!(
                (a - phi).abs() < 0.02,
                "planted phi={phi} recovered as {a:.4}"
            );
        }
    }

    #[test]
    fn planted_hour_drift_concentrates_in_its_hour() {
        let mut s = spec(Generator::PlantedHourDrift, 8, 24 * 400);
        s.params.drift_hour = 14;
        let r = returns(&s).unwrap().0;
        let in_hour: Vec<f64> = r.iter().copied().step_by(24).skip(14).collect();
        // Bar i closes at hour (i % 24), so hour 14 is every 24th from index 14.
        let hour14: Vec<f64> = r.iter().skip(14).step_by(24).copied().collect();
        let other: Vec<f64> = r
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 24 != 14)
            .map(|(_, x)| *x)
            .collect();
        assert!(!in_hour.is_empty());
        assert!(
            mean(&hour14) > mean(&other) + 2.0 * stdev(&hour14) / (hour14.len() as f64).sqrt(),
            "hour-14 mean {:.6} is not above the rest {:.6}",
            mean(&hour14),
            mean(&other)
        );
    }

    #[test]
    fn planted_carry_series_matches_the_drift_it_produced() {
        let s = spec(Generator::PlantedCarry, 17, 20_000);
        let r = returns(&s).unwrap().0;
        let c = carry_series(&s);
        assert_eq!(c.len(), r.len());
        // Regress returns on the (lagged, i.e. same-index) carry; the slope must
        // recover carry_beta.
        let cm = mean(&c);
        let rm = mean(&r);
        let num: f64 = c.iter().zip(&r).map(|(x, y)| (x - cm) * (y - rm)).sum();
        let den: f64 = c.iter().map(|x| (x - cm).powi(2)).sum();
        let slope = num / den;
        assert!(
            (slope - s.params.carry_beta).abs() < 0.0008,
            "carry beta {} recovered as {slope:.6}",
            s.params.carry_beta
        );
    }

    #[test]
    fn bars_are_internally_consistent() {
        let (bars, _) = generate(&spec(Generator::GarchT, 3, 1000)).unwrap();
        for b in &bars {
            let (o, h, l, c) = (
                b.open.parse::<f64>().unwrap(),
                b.high.parse::<f64>().unwrap(),
                b.low.parse::<f64>().unwrap(),
                b.close.parse::<f64>().unwrap(),
            );
            assert!(h >= o.max(c) - 1e-9, "high below the body");
            assert!(l <= o.min(c) + 1e-9, "low above the body");
            assert!(l > 0.0, "non-positive price");
        }
        // Bars are contiguous on the hour.
        for w in bars.windows(2) {
            assert_eq!(
                (w[1].available_time - w[0].available_time).num_seconds(),
                3600
            );
        }
    }

    /// The wick must not leak the next bar. If it did, every generator would
    /// contain an edge and the `noise` suite would be unscoreable.
    #[test]
    fn wicks_do_not_predict_the_next_bar() {
        let (bars, _) = generate(&spec(Generator::GarchT, 11, 20_000)).unwrap();
        let range: Vec<f64> = bars
            .iter()
            .map(|b| b.high.parse::<f64>().unwrap() - b.low.parse::<f64>().unwrap())
            .collect();
        let next: Vec<f64> = bars
            .windows(2)
            .map(|w| w[1].close.parse::<f64>().unwrap() / w[0].close.parse::<f64>().unwrap() - 1.0)
            .collect();
        let x = &range[..next.len()];
        let (xm, ym) = (mean(x), mean(&next));
        let cov: f64 = x
            .iter()
            .zip(&next)
            .map(|(a, b)| (a - xm) * (b - ym))
            .sum::<f64>()
            / x.len() as f64;
        let corr = cov / (stdev(x) * stdev(&next));
        assert!(
            corr.abs() < 0.05,
            "bar range predicts the next return at {corr}"
        );
    }

    #[test]
    fn has_edge_matches_the_truth_descriptor() {
        for g in Generator::ALL {
            let (_, truth) = returns(&spec(*g, 1, 64)).unwrap();
            assert_eq!(
                truth.exploitable,
                g.has_edge(),
                "{g}: has_edge and Truth.exploitable disagree"
            );
            assert_eq!(truth.mechanism == "none", !g.has_edge());
        }
    }

    #[test]
    fn instrument_ids_follow_the_spec() {
        assert_eq!(
            spec(Generator::PlantedAr1, 42, 10).instrument_id(),
            "SYN-PLANTED-AR1-42"
        );
        assert_eq!(
            spec(Generator::GarchT, 7, 10).instrument_id(),
            "SYN-GARCH-T-7"
        );
    }

    #[test]
    fn absurd_requests_are_refused_before_anything_is_generated() {
        let mut s = spec(Generator::GarchT, 1, MAX_LENGTH + 1);
        assert!(matches!(returns(&s), Err(SyntheticError::BadLength { .. })));
        s.length = 100;
        s.timeframe = "3m".into();
        assert!(matches!(returns(&s), Err(SyntheticError::BadTimeframe(_))));
        s.timeframe = "1h".into();
        s.params.alpha = 0.6;
        s.params.beta = 0.6;
        assert!(matches!(returns(&s), Err(SyntheticError::BadParam { .. })));
    }
}
