//! What a finished run's own trades still have to say (AGENT-006 §5.5; Set S
//! task S-3.12).
//!
//! Two tools, and neither of them runs anything. Both read the trade list of a
//! run that already happened — through `/api/evidence/trades`, the same route
//! `get_trade_list` uses and therefore the same ownership check — and re-read it
//! a different way.
//!
//! # Why neither spends a trial
//!
//! A trial is a *look at the market*, and the look these tools re-read was
//! charged when the run was dispatched. Re-ordering the trades that look produced,
//! or re-charging their costs, adds no observation: the same fills over the same
//! bars. That is INV-18, and it is the reasoning `backtest::stats::bootstrap`
//! states for the stationary bootstrap — a statistic computed from an existing
//! look is not a new look, and counting it as one would deflate the platform
//! against work it never did. AGENT-006 §5.5 declares both `none`, and the
//! declaration is what [`crate::gate::CallGate`] enforces at the door.
//!
//! Getting this wrong in the other direction would be just as bad: charging a
//! falsification trial for re-reading a stored array raises N_eff against work
//! nobody did, and every selection-adjusted statistic downstream divides by that
//! count.
//!
//! # Arithmetic here, data over there
//!
//! ADR-0010 makes this crate a thin HTTP client, and it stays one: nothing below
//! opens a database, spawns a process or touches the parquet cache. What it does
//! is arithmetic over a response it was handed, which is what `compare_backtests`
//! in [`crate::tools::backtests`] already does one run at a time.
//!
//! The one thing deliberately *not* computed here is anything a gate also
//! computes. [`crate::tools::audit`]'s module docs give the reason: a tool that
//! agrees with the gate by coincidence will disagree with it eventually. Nothing
//! in this file has a counterpart in `backtest::gates` — drawdown percentiles
//! over resampled trade paths and a break-even cost multiple are produced nowhere
//! else on the platform.
//!
//! # What is not offered, and what it would take
//!
//! `trade_reshuffle_bootstrap` offers `reshuffle` and `bootstrap`. AGENT-006
//! §5.5 also names `block`, and it is absent rather than approximated: a block
//! bootstrap needs an expected block length estimated from the series' own
//! autocorrelation, `backtest::stats::bootstrap::politis_white_block` is the
//! platform's implementation of that estimate, and this crate does not depend on
//! `backtest`. A second Politis-White written here would be a second answer to a
//! question the platform already answers, differing from it silently. The schema
//! refuses `block` by name and the refusal says which symbol is missing.

use serde_json::{json, Value};

use crate::spec::{ToolSpec, TrialKind};
use crate::taxonomy::Risk;
use crate::ApiClient;

// ── Refusal thresholds ───────────────────────────────────────────────────────

/// Below this many closed trades, a percentile over resampled paths is a number
/// with no distribution behind it.
///
/// The same floor `backtest::stats::bootstrap` applies to a return series, for
/// the reason it gives there: a 5th-percentile statistic from twelve
/// observations is a number, and it is not evidence.
pub const MIN_TRADES: usize = 30;

/// §12.3 Gate 13's floor, applied here because the percentile means the same
/// thing: below a thousand paths the tail moves with the seed.
pub const MIN_PATHS: usize = 1_000;

/// Above this the tool burns wall-clock for digits nobody reads.
pub const MAX_PATHS: usize = 50_000;

/// AGENT-006 §5.5's default path count.
const DEFAULT_PATHS: usize = 5_000;

/// The multiplier grid AGENT-006 §5.5 names. Multiples of the run's **own**
/// charged costs: there is no basis-point figure in it, which is the point.
const DEFAULT_MULTIPLIERS: &[f64] = &[1.0, 1.5, 2.0, 3.0, 5.0];

// ── Shared plumbing ──────────────────────────────────────────────────────────

fn missing(field: &str) -> Value {
    json!({ "error": "missing_field", "field": field })
}

fn str_field(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The evidence route's own number reader, mirrored.
///
/// A `Decimal` reaches JSON as a string under some serde configurations and as a
/// number under others, and `crates/api/src/routes/evidence.rs` already reads
/// both. Reading it one way here would make this tool disagree with
/// `get_trade_list` about the same array.
fn as_number(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

/// One closed trade, as much of it as these two tools need.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TradeRow {
    /// Realised P&L **net of costs**, in the run's quote currency.
    pub pnl: f64,
    /// Costs charged on this trade, same currency. `None` when the field is
    /// absent — never zero, because unrecorded costs and zero costs give
    /// opposite answers to "what would twice the costs have done".
    pub costs_paid: Option<f64>,
    /// `|entry_price * qty| + |exit_price * qty|`, the notional a basis-point fee
    /// is charged on. `None` when any of the three is absent.
    pub notional: Option<f64>,
}

fn read_trades(payload: &Value) -> Vec<TradeRow> {
    payload
        .get("trades")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|t| {
                    let leg = |price: &str| {
                        let p = t.get(price).and_then(as_number)?;
                        let q = t.get("qty").and_then(as_number)?;
                        Some((p * q).abs())
                    };
                    TradeRow {
                        pnl: t.get("pnl").and_then(as_number).unwrap_or(0.0),
                        costs_paid: t.get("costs_paid").and_then(as_number),
                        notional: match (leg("entry_price"), leg("exit_price")) {
                            (Some(a), Some(b)) => Some(a + b),
                            _ => None,
                        },
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Fetch the run's trades, or the refusal to return instead of them.
async fn fetch_trades(api: &ApiClient, run_id: &str) -> Result<Vec<TradeRow>, Value> {
    let payload = match api
        .post(
            "/api/evidence/trades",
            // `top_n` only steers the concentration block, which neither tool
            // reads; 1 is the cheapest value the route accepts.
            json!({ "run_id": run_id, "top_n": 1 }),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => return Err(e.to_tool_error()),
    };
    if payload.get("error").is_some() {
        return Err(payload);
    }
    let trades = read_trades(&payload);
    if trades.is_empty() {
        return Err(json!({
            "error": "no_trades",
            "run_id": run_id,
            "why": "this run recorded no closed trades, so there is no sequence to resample \
                    and no cost to re-charge. An empty trade list is not a result with zero \
                    risk; it is a run that did not trade.",
            "fix": "get_backtest shows whether the run completed and whether its strategy \
                    produced any entries"
        }));
    }
    Ok(trades)
}

// ── A deterministic generator ────────────────────────────────────────────────

/// SplitMix64, self-contained.
///
/// `backtest::rng::DetRng` is the platform's generator and this crate does not
/// depend on `backtest`. Determinism is not optional here even so: a resampling
/// tool whose answer moved between calls could not be cited as evidence, so the
/// seed is an input and the same seed gives the same percentiles.
struct Rng(u64);

impl Rng {
    const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

    const fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(Self::GAMMA);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform on `0..n`, rejecting the biased tail rather than taking a modulo
    /// of the whole range.
    fn below(&mut self, n: usize) -> usize {
        debug_assert!(n > 0, "below(0) has no value");
        let n64 = n as u64;
        let limit = u64::MAX - (u64::MAX % n64);
        loop {
            let x = self.next_u64();
            if x < limit {
                return (x % n64) as usize;
            }
        }
    }
}

// ── Path statistics ──────────────────────────────────────────────────────────

/// What one ordering of the trades did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathStats {
    /// Largest peak-to-trough fall of cumulative P&L, in money. `>= 0`.
    pub max_drawdown: f64,
    /// Longest run of consecutive trades spent below the running peak.
    pub time_under_water: usize,
    /// Cumulative P&L at the end of the path.
    pub endpoint: f64,
    /// Lowest cumulative P&L the path reached. `<= 0`, because the path starts
    /// at zero and a capital level is applied afterwards.
    pub min_cumulative: f64,
}

/// Walk one ordering.
#[must_use]
pub fn walk(pnl: &[f64]) -> PathStats {
    let mut cumulative = 0.0_f64;
    let mut peak = 0.0_f64;
    let mut max_drawdown = 0.0_f64;
    let mut min_cumulative = 0.0_f64;
    let mut under = 0_usize;
    let mut longest_under = 0_usize;
    for &p in pnl {
        cumulative += p;
        if cumulative >= peak {
            peak = cumulative;
            under = 0;
        } else {
            under += 1;
            if under > longest_under {
                longest_under = under;
            }
        }
        let drawdown = peak - cumulative;
        if drawdown > max_drawdown {
            max_drawdown = drawdown;
        }
        if cumulative < min_cumulative {
            min_cumulative = cumulative;
        }
    }
    PathStats {
        max_drawdown,
        time_under_water: longest_under,
        endpoint: cumulative,
        min_cumulative,
    }
}

/// Nearest-rank percentile of an already-sorted slice. `q` in `0.0..=1.0`.
fn percentile(sorted: &[f64], q: f64) -> f64 {
    debug_assert!(!sorted.is_empty(), "the percentile of nothing has no value");
    let n = sorted.len();
    let rank = (q * n as f64).ceil() as usize;
    sorted[rank.clamp(1, n) - 1]
}

const LOSS_QS: &[(&str, f64)] = &[("p50", 0.50), ("p90", 0.90), ("p95", 0.95), ("p99", 0.99)];

fn pctile_block(sorted: &[f64]) -> Value {
    let mut o = serde_json::Map::new();
    for (label, q) in LOSS_QS {
        o.insert((*label).to_string(), json!(percentile(sorted, *q)));
    }
    Value::Object(o)
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

fn std_dev(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = mean(v);
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64).sqrt()
}

// ── S-3.12a: `trade_reshuffle_bootstrap` ─────────────────────────────────────

/// The two questions the tool can answer, which are not the same question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Permute the realised outcomes. The set of trades is held fixed, so the
    /// endpoint is fixed with it and only the *path* varies.
    Reshuffle,
    /// Resample the realised outcomes with replacement. The set of trades
    /// varies, so the endpoint varies with it.
    Bootstrap,
}

impl Mode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reshuffle => "reshuffle",
            Self::Bootstrap => "bootstrap",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "reshuffle" => Some(Self::Reshuffle),
            "bootstrap" => Some(Self::Bootstrap),
            _ => None,
        }
    }

    /// What this mode's output is about, carried in every result.
    ///
    /// Conflating the two is the documented error this tool exists around, and a
    /// caller who cannot see which one ran cannot avoid it.
    const fn measures(self) -> &'static str {
        match self {
            Self::Reshuffle => {
                "path risk of a FIXED outcome set: these exact trades in a different order. It \
                 answers 'could the equity curve have looked much worse along the way', and it \
                 assumes the trades themselves are exactly right."
            }
            Self::Bootstrap => {
                "estimation uncertainty: what a strategy with this trade distribution might have \
                 produced over a sample of this length. The outcome set is resampled, so the \
                 endpoint moves and the dispersion is a statement about the endpoint."
            }
        }
    }
}

/// Everything the resampler needs, parsed and bounded.
#[derive(Clone, Debug, PartialEq)]
pub struct ResampleRequest {
    pub run_id: String,
    pub mode: Mode,
    pub n_paths: usize,
    pub seed: u64,
    /// `None` means no ruin probability is reported, and the result says so.
    pub capital_levels: Option<Vec<f64>>,
}

/// Parse the call, or the refusal to return instead.
///
/// Separate from the handler so every refusal is reachable without a live API
/// client: a refusal nothing tests is a refusal that silently becomes a `None`
/// somewhere.
fn parse_resample(params: &Value) -> Result<ResampleRequest, Value> {
    let Some(run_id) = str_field(params, "run_id") else {
        return Err(missing("run_id"));
    };
    // No default. Reshuffle and bootstrap answer different questions and the
    // documented failure is reporting one as the other; a default would pick the
    // question for a caller who had not noticed there were two.
    let Some(raw_mode) = str_field(params, "mode") else {
        return Err(json!({
            "error": "mode_required",
            "why": "reshuffle and bootstrap answer different questions and this tool will not \
                    choose for you. Reshuffle holds the outcome set fixed and varies the ORDER, \
                    so it measures path risk. Bootstrap resamples the outcomes, so it measures \
                    estimation uncertainty about the endpoint.",
            "accepts": ["reshuffle", "bootstrap"]
        }));
    };
    let Some(mode) = Mode::parse(&raw_mode) else {
        return Err(json!({
            "error": "unknown_mode",
            "mode": raw_mode,
            "accepts": ["reshuffle", "bootstrap"],
            "block_not_offered": "a block bootstrap needs an expected block length estimated \
                                  from the series' own autocorrelation. The platform's estimator \
                                  is `backtest::stats::bootstrap::politis_white_block`, this \
                                  crate cannot reach it, and a second estimator written here \
                                  would disagree with that one silently."
        }));
    };

    let n_paths = params
        .get("n_paths")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_PATHS, |n| usize::try_from(n).unwrap_or(usize::MAX));
    if !(MIN_PATHS..=MAX_PATHS).contains(&n_paths) {
        return Err(json!({
            "error": "n_paths_out_of_range",
            "asked": n_paths,
            "min": MIN_PATHS,
            "max": MAX_PATHS,
            "why": "below the minimum the tail percentiles move with the seed, which makes them \
                    a property of the draw rather than of the strategy"
        }));
    }

    // A capital level of zero or less cannot be ruined into, so it is dropped
    // rather than reported as a probability of one.
    let capital_levels: Option<Vec<f64>> = params
        .get("capital_levels")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(as_number)
                .filter(|c| *c > 0.0)
                .collect::<Vec<f64>>()
        })
        .filter(|v| !v.is_empty());

    Ok(ResampleRequest {
        run_id,
        mode,
        n_paths,
        seed: params.get("seed").and_then(Value::as_u64).unwrap_or(0),
        capital_levels,
    })
}

pub async fn trade_reshuffle_bootstrap(api: &ApiClient, params: &Value) -> Value {
    let req = match parse_resample(params) {
        Ok(r) => r,
        Err(refusal) => return refusal,
    };
    let trades = match fetch_trades(api, &req.run_id).await {
        Ok(t) => t,
        Err(refusal) => return refusal,
    };
    resample_report(&trades, &req)
}

/// The whole answer, given the trades. Pure, so the refusals and the arithmetic
/// are testable without a network.
fn resample_report(trades: &[TradeRow], req: &ResampleRequest) -> Value {
    if trades.len() < MIN_TRADES {
        return json!({
            "error": "too_few_trades",
            "run_id": req.run_id,
            "have": trades.len(),
            "need": MIN_TRADES,
            "why": "a percentile over resampled paths needs a sample to resample. Below this the \
                    answer is a property of the handful of trades that happened, and reporting \
                    it as a distribution would dress an anecdote as a statistic.",
            "fix": "run over a longer window, or at a timeframe that trades more often"
        });
    }

    let pnl: Vec<f64> = trades.iter().map(|t| t.pnl).collect();
    let observed = walk(&pnl);
    let sim = simulate(&pnl, req);

    let mut out = json!({
        "run_id": req.run_id,
        "mode": req.mode.as_str(),
        "measures": req.mode.measures(),
        "n_trades": trades.len(),
        "n_paths": req.n_paths,
        "seed": req.seed,
        "units": "the run's quote currency; drawdowns and endpoints are money, not fractions",
        "observed": {
            "max_drawdown": observed.max_drawdown,
            "time_under_water_trades": observed.time_under_water,
            "endpoint": observed.endpoint
        },
        "max_drawdown_pctiles": sim.max_drawdown,
        "time_under_water_pctiles_trades": sim.time_under_water
    });
    let o = out.as_object_mut().expect("built as an object");

    match sim.endpoint_dispersion {
        Some(d) => {
            o.insert("endpoint_dispersion".into(), d);
        }
        None => {
            // Absent, with the reason. A zero here would read as "the endpoint is
            // certain", which is the opposite of what a reshuffle establishes —
            // it establishes nothing about the endpoint at all.
            o.insert("endpoint_dispersion".into(), Value::Null);
            o.insert(
                "endpoint_dispersion_absent_because".into(),
                json!(
                    "a reshuffle permutes a fixed set of outcomes, so every path ends at the \
                     same total by construction. That is arithmetic, not evidence that the \
                     endpoint is reliable, so no dispersion is reported."
                ),
            );
            o.insert(
                "not_a_worst_case".into(),
                json!(
                    "this is NOT a worst case and NOT a capital requirement. It is the range of \
                     paths through the outcomes this run happened to produce; a worse market \
                     would have produced worse outcomes, and no reordering of these can show \
                     that. Use `mode: bootstrap` for a statement about the endpoint."
                ),
            );
        }
    }

    match sim.ruin {
        Some(r) => {
            o.insert("ruin_prob_by_capital".into(), r);
            o.insert(
                "ruin_prob_interpretation".into(),
                json!(match req.mode {
                    Mode::Reshuffle =>
                        "the fraction of orderings OF THESE TRADES that would have reached zero \
                         equity from the given starting capital. Conditional on this outcome set \
                         being exactly right, which it is not — so it is a floor on path risk, \
                         not a capital requirement.",
                    Mode::Bootstrap =>
                        "the fraction of resampled histories, of the same length as this run, \
                         that reached zero equity from the given starting capital.",
                }),
            );
        }
        None => {
            o.insert("ruin_prob_by_capital".into(), Value::Null);
            o.insert(
                "ruin_prob_absent_because".into(),
                json!(
                    "no positive `capital_levels` were supplied, and a ruin probability has no \
                     value without a capital level to be ruined from"
                ),
            );
        }
    }
    out
}

struct Simulation {
    max_drawdown: Value,
    time_under_water: Value,
    endpoint_dispersion: Option<Value>,
    ruin: Option<Value>,
}

fn simulate(pnl: &[f64], req: &ResampleRequest) -> Simulation {
    let n = pnl.len();
    let mut rng = Rng::new(req.seed);
    let mut scratch = vec![0.0_f64; n];

    let mut drawdowns = Vec::with_capacity(req.n_paths);
    let mut unders = Vec::with_capacity(req.n_paths);
    let mut endpoints = Vec::with_capacity(req.n_paths);
    let mut troughs = Vec::with_capacity(req.n_paths);

    for _ in 0..req.n_paths {
        match req.mode {
            Mode::Reshuffle => {
                scratch.copy_from_slice(pnl);
                // Fisher-Yates.
                for i in (1..n).rev() {
                    let j = rng.below(i + 1);
                    scratch.swap(i, j);
                }
            }
            Mode::Bootstrap => {
                for slot in scratch.iter_mut() {
                    *slot = pnl[rng.below(n)];
                }
            }
        }
        let s = walk(&scratch);
        drawdowns.push(s.max_drawdown);
        unders.push(s.time_under_water as f64);
        endpoints.push(s.endpoint);
        troughs.push(s.min_cumulative);
    }

    drawdowns.sort_by(f64::total_cmp);
    unders.sort_by(f64::total_cmp);

    let ruin = req.capital_levels.as_ref().map(|levels| {
        Value::Array(
            levels
                .iter()
                .map(|c| {
                    // Ruined when the deepest point of the path takes equity to
                    // zero or below.
                    let hits = troughs.iter().filter(|t| **t <= -*c).count();
                    json!({ "capital": c, "prob": hits as f64 / req.n_paths as f64 })
                })
                .collect(),
        )
    });

    let endpoint_dispersion = match req.mode {
        Mode::Reshuffle => None,
        Mode::Bootstrap => {
            endpoints.sort_by(f64::total_cmp);
            Some(json!({
                "p5": percentile(&endpoints, 0.05),
                "p50": percentile(&endpoints, 0.50),
                "p95": percentile(&endpoints, 0.95),
                "mean": mean(&endpoints),
                "sd": std_dev(&endpoints),
                "prob_endpoint_negative":
                    endpoints.iter().filter(|x| **x < 0.0).count() as f64 / endpoints.len() as f64
            }))
        }
    };

    Simulation {
        max_drawdown: pctile_block(&drawdowns),
        time_under_water: pctile_block(&unders),
        endpoint_dispersion,
        ruin,
    }
}

// ── S-3.12b: `cost_shock_curve` ──────────────────────────────────────────────

/// Everything the cost curve needs, parsed and bounded.
#[derive(Clone, Debug, PartialEq)]
pub struct ShockRequest {
    pub run_id: String,
    /// Multiples of the run's **own** charged costs, never of a quoted spread.
    pub multipliers: Vec<f64>,
    /// A *measured* live cost, when the caller has one. `None` means no point on
    /// the curve is claimed to be where live sits.
    pub live_cost_bps: Option<f64>,
}

fn parse_shock(params: &Value) -> Result<ShockRequest, Value> {
    let Some(run_id) = str_field(params, "run_id") else {
        return Err(missing("run_id"));
    };
    let multipliers: Vec<f64> = match params.get("multipliers").and_then(Value::as_array) {
        Some(a) => {
            let parsed: Vec<f64> = a
                .iter()
                .filter_map(as_number)
                .filter(|m| *m >= 0.0 && m.is_finite())
                .collect();
            if parsed.len() != a.len() {
                return Err(json!({
                    "error": "bad_multipliers",
                    "why": "every multiplier must be a finite, non-negative number: this grid is \
                            made of multiples of the run's measured costs"
                }));
            }
            parsed
        }
        None => DEFAULT_MULTIPLIERS.to_vec(),
    };
    if multipliers.is_empty() {
        return Err(json!({
            "error": "bad_multipliers",
            "why": "the grid is empty, so there is no curve to report"
        }));
    }
    Ok(ShockRequest {
        run_id,
        multipliers,
        live_cost_bps: params
            .get("live_cost_bps")
            .and_then(as_number)
            .filter(|b| b.is_finite()),
    })
}

pub async fn cost_shock_curve(api: &ApiClient, params: &Value) -> Value {
    let req = match parse_shock(params) {
        Ok(r) => r,
        Err(refusal) => return refusal,
    };
    let trades = match fetch_trades(api, &req.run_id).await {
        Ok(t) => t,
        Err(refusal) => return refusal,
    };
    shock_report(&trades, &req)
}

fn shock_report(trades: &[TradeRow], req: &ShockRequest) -> Value {
    // Unrecorded costs are not zero costs. Treating a missing field as zero
    // produces an infinite break-even multiple — the most reassuring number this
    // tool could return, and it would mean the opposite of what it says.
    let unrecorded = trades.iter().filter(|t| t.costs_paid.is_none()).count();
    if unrecorded > 0 {
        return json!({
            "error": "costs_not_recorded",
            "run_id": req.run_id,
            "trades_missing_costs_paid": unrecorded,
            "of": trades.len(),
            "why": "not every trade in this run carries `costs_paid`, and a missing cost is not \
                    a zero cost. Scaling an unknown base would return a break-even multiple \
                    saying the strategy survives any cost, which is the most dangerous wrong \
                    answer available here.",
            "fix": "re-run under a cost model that records per-trade costs"
        });
    }

    let total_costs: f64 = trades.iter().filter_map(|t| t.costs_paid).sum();
    let net_pnl: f64 = trades.iter().map(|t| t.pnl).sum();
    // `pnl` is net of costs, so the gross is recoverable exactly.
    let gross_pnl = net_pnl + total_costs;

    if total_costs <= 0.0 {
        return json!({
            "error": "no_costs_charged",
            "run_id": req.run_id,
            "total_costs": total_costs,
            "why": "this run was charged no costs at all, so there is nothing to multiply and \
                    every point on the curve would equal the unshocked result. A zero-cost \
                    backtest is itself the finding — the break-even multiple would be infinite, \
                    and it would be a fact about the cost model rather than about the strategy.",
            "fix": "run_oracle_test measures whether the cost model charges what it claims"
        });
    }

    let curve: Vec<Value> = req
        .multipliers
        .iter()
        .map(|m| {
            let net = gross_pnl - m * total_costs;
            json!({
                "multiplier": m,
                "costs_charged": m * total_costs,
                "net_pnl": net,
                "profitable": net > 0.0
            })
        })
        .collect();

    // Where the net reaches zero: gross - m * total_costs = 0.
    let breakeven = gross_pnl / total_costs;

    // Notional is optional on the wire, so the basis-point view is optional in
    // the result rather than defaulted to something.
    let total_notional: Option<f64> = trades
        .iter()
        .map(|t| t.notional)
        .try_fold(0.0_f64, |acc, n| n.map(|v| acc + v))
        .filter(|v| *v > 0.0);
    let baseline_bps = total_notional.map(|n| total_costs / n * 10_000.0);

    let mut out = json!({
        "run_id": req.run_id,
        "curve": curve,
        "breakeven_multiplier": breakeven,
        "already_unprofitable": net_pnl <= 0.0,
        "baseline": {
            "total_costs": total_costs,
            "net_pnl_at_1x": net_pnl,
            "gross_pnl_before_costs": gross_pnl,
            "n_trades": trades.len()
        },
        "grid_origin": "multiples of the costs THIS RUN actually charged, trade by trade \
                        (`Trade.costs_paid`) — not a textbook basis-point figure and not a \
                        venue's published schedule. Whatever the run's cost model charged is \
                        the 1x point.",
        "assumes_fixed_trade_set": true,
        "what_that_assumption_costs": "the same trades are re-charged, so nothing here models a \
                                       strategy that would have traded less under higher costs, \
                                       nor the worse fills that usually arrive with them. This \
                                       is a re-pricing of a fixed decision sequence, not a \
                                       re-optimisation.",
        "units": "the run's quote currency"
    });
    let o = out.as_object_mut().expect("built as an object");

    match baseline_bps {
        Some(bps) => {
            o.insert("baseline_cost_bps_of_notional".into(), json!(bps));
        }
        None => {
            o.insert("baseline_cost_bps_of_notional".into(), Value::Null);
            o.insert(
                "baseline_cost_bps_absent_because".into(),
                json!(
                    "not every trade carries `entry_price`, `exit_price` and `qty`, so the traded \
                     notional is unknown and the charged cost cannot be expressed in basis \
                     points of it"
                ),
            );
        }
    }

    match (req.live_cost_bps, baseline_bps) {
        (Some(live), Some(base)) if base > 0.0 => {
            let implied = live / base;
            o.insert(
                "live_cost".into(),
                json!({
                    "live_cost_bps": live,
                    "implied_multiplier": implied,
                    "safety_margin": breakeven - implied,
                    "survives_live_costs": implied < breakeven
                }),
            );
        }
        (Some(live), _) => {
            o.insert("live_cost".into(), Value::Null);
            o.insert(
                "live_cost_absent_because".into(),
                json!(format!(
                    "`live_cost_bps` was {live}, but this run's charged cost cannot be expressed \
                     in basis points because the traded notional is not recorded. The two are \
                     not on a common scale, so no safety margin is computed."
                )),
            );
        }
        (None, _) => {
            o.insert("live_cost".into(), Value::Null);
            o.insert(
                "live_cost_absent_because".into(),
                json!(
                    "no `live_cost_bps` was supplied. The curve says what the result becomes at \
                     each multiple of the run's own costs; where live costs actually sit on it \
                     is a measurement this tool does not have and will not guess."
                ),
            );
        }
    }
    out
}

// ── The catalogue entries ────────────────────────────────────────────────────

/// Both tools declare [`TrialKind::None`], and that is the load-bearing line in
/// this file.
///
/// The declaration is what [`crate::gate::CallGate`] reads before a handler runs.
/// Declaring a trial here would charge falsification budget for re-reading a
/// stored array, inflating the N_eff every selection-adjusted statistic divides
/// by. Declaring none for a tool that *did* dispatch a run would be an
/// unrecorded look. Neither of these dispatches anything: their only outbound
/// call is a read of `/api/evidence/trades`, the same route `get_trade_list`
/// already spends nothing on.
#[must_use]
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            "trade_reshuffle_bootstrap",
            "falsification",
            Risk::Read,
            "ASK WHETHER THE EQUITY CURVE COULD HAVE LOOKED FAR WORSE, by reordering \
             (`reshuffle`) or resampling (`bootstrap`) the run's own trades. Spends no trial; \
             `mode` is required because the two answer different questions.",
            json!({
                "type": "object",
                "required": ["run_id", "mode"],
                "properties": {
                    "run_id": { "type": "string" },
                    "mode": {
                        "type": "string",
                        "enum": ["reshuffle", "bootstrap"],
                        "description": "`reshuffle` is path risk of a fixed outcome set; `bootstrap` is endpoint uncertainty. `block` is not offered."
                    },
                    "n_paths": { "type": "integer", "description": "1000-50000, default 5000." },
                    "capital_levels": {
                        "type": "array",
                        "items": { "type": "number" },
                        "description": "Starting capitals to report a ruin probability from. Omit and none is reported."
                    },
                    "seed": { "type": "integer", "description": "Default 0. Fix it; do not vary it." }
                }
            }),
        )
        .internal()
        // Stated rather than left to the default. `TrialKind::None` is the
        // default, and a correctness claim that reads as an omission is a claim
        // the next editor will change without noticing they changed it.
        .spends_trial(TrialKind::None)
        .evidence(&["run_id"])
        .notes("trade_reshuffle_bootstrap"),
        ToolSpec::new(
            "cost_shock_curve",
            "falsification",
            Risk::Read,
            "ASK HOW MUCH HIGHER THE COSTS COULD HAVE BEEN before this result turns negative. \
             Re-prices the run's own trades at multiples of the costs it was actually charged, \
             and spends no trial.",
            json!({
                "type": "object",
                "required": ["run_id"],
                "properties": {
                    "run_id": { "type": "string" },
                    "multipliers": {
                        "type": "array",
                        "items": { "type": "number" },
                        "description": "Multiples of the run's measured costs. Default 1, 1.5, 2, 3, 5."
                    },
                    "live_cost_bps": {
                        "type": "number",
                        "description": "A MEASURED live cost, if you have one, to place on the curve. Omit it rather than estimating one."
                    }
                }
            }),
        )
        .internal()
        .spends_trial(TrialKind::None)
        .evidence(&["run_id"])
        .notes("cost_shock_curve"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Twenty-four wins and twelve losses of six different sizes: thirty-six
    /// trades, so it clears [`MIN_TRADES`], and enough losses that reordering
    /// them genuinely changes how deep the curve goes.
    ///
    /// The shape is load-bearing. A fixture with a *single* loss has exactly the
    /// same maximum drawdown in every permutation — the peak before the loss
    /// minus the trough after it is the loss, wherever it falls — so a test
    /// asserting that reshuffling spreads the drawdown would pass over it while
    /// measuring nothing.
    ///
    /// Gains total 240, losses total 210, so the endpoint is +30 and no ordering
    /// can take the trough below -210.
    fn series() -> Vec<f64> {
        let mut v = vec![10.0_f64; 24];
        for _ in 0..2 {
            v.extend_from_slice(&[-5.0, -10.0, -15.0, -20.0, -25.0, -30.0]);
        }
        v
    }

    /// Every trade a loss, so the trough is -30 in every ordering and a ruin
    /// probability is exactly one or exactly zero rather than nearly so.
    fn all_losses() -> Vec<f64> {
        vec![-1.0_f64; 30]
    }

    fn rows(pnl: &[f64], costs: f64) -> Vec<TradeRow> {
        pnl.iter()
            .map(|p| TradeRow {
                pnl: *p,
                costs_paid: Some(costs),
                notional: Some(1_000.0),
            })
            .collect()
    }

    fn req(mode: Mode, capital: Option<Vec<f64>>) -> ResampleRequest {
        ResampleRequest {
            run_id: "r1".into(),
            mode,
            n_paths: MIN_PATHS,
            seed: 0,
            capital_levels: capital,
        }
    }

    fn f(v: &Value, path: &[&str]) -> f64 {
        let mut cur = v;
        for k in path {
            cur = cur.get(k).unwrap_or_else(|| panic!("missing {k} in {v}"));
        }
        cur.as_f64()
            .unwrap_or_else(|| panic!("{path:?} is not a number in {v}"))
    }

    // ── walk ─────────────────────────────────────────────────────────────────

    #[test]
    fn walk_measures_the_fall_from_the_peak_not_from_the_start() {
        // Up 10, up 10, down 15, up 1: peak 20, trough 5, so the drawdown is 15
        // and two trades are spent under water.
        let s = walk(&[10.0, 10.0, -15.0, 1.0]);
        assert!((s.max_drawdown - 15.0).abs() < 1e-9, "{s:?}");
        assert_eq!(s.time_under_water, 2, "{s:?}");
        assert!((s.endpoint - 6.0).abs() < 1e-9, "{s:?}");
        // The path never went below where it started, so the trough is zero.
        assert!(s.min_cumulative.abs() < 1e-9, "{s:?}");
    }

    #[test]
    fn walk_records_a_trough_below_the_starting_line() {
        let s = walk(&[-5.0, -3.0, 20.0]);
        assert!((s.min_cumulative + 8.0).abs() < 1e-9, "{s:?}");
        assert!((s.max_drawdown - 8.0).abs() < 1e-9, "{s:?}");
    }

    #[test]
    fn time_under_water_is_the_longest_spell_not_the_last_one() {
        // A three-trade dip, then a one-trade dip.
        let s = walk(&[10.0, -1.0, -1.0, -1.0, 10.0, -1.0, 10.0]);
        assert_eq!(s.time_under_water, 3, "{s:?}");
    }

    #[test]
    fn percentile_is_nearest_rank_and_never_indexes_out() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert!((percentile(&v, 0.50) - 50.0).abs() < 1e-9);
        assert!((percentile(&v, 0.99) - 99.0).abs() < 1e-9);
        assert!((percentile(&v, 0.0) - 1.0).abs() < 1e-9);
        assert!((percentile(&v, 1.0) - 100.0).abs() < 1e-9);
    }

    // ── the generator ────────────────────────────────────────────────────────

    #[test]
    fn the_same_seed_gives_the_same_paths() {
        let pnl = series();
        let a = simulate(&pnl, &req(Mode::Bootstrap, Some(vec![100.0])));
        let b = simulate(&pnl, &req(Mode::Bootstrap, Some(vec![100.0])));
        assert_eq!(a.max_drawdown, b.max_drawdown);
        assert_eq!(a.endpoint_dispersion, b.endpoint_dispersion);
        assert_eq!(a.ruin, b.ruin);
        // And it measured something: a simulation producing no numbers at all
        // would satisfy every equality above.
        assert!(
            f(&a.max_drawdown, &["p99"]) > 0.0,
            "the p99 drawdown of a series containing twelve losses must be positive: {:?}",
            a.max_drawdown
        );
    }

    #[test]
    fn a_different_seed_gives_different_paths() {
        let pnl = series();
        let mut one = req(Mode::Bootstrap, None);
        one.seed = 1;
        let mut two = req(Mode::Bootstrap, None);
        two.seed = 2;
        assert_ne!(
            simulate(&pnl, &one).endpoint_dispersion,
            simulate(&pnl, &two).endpoint_dispersion
        );
    }

    #[test]
    fn below_draws_every_value_in_range() {
        let mut rng = Rng::new(3);
        let mut seen = [false; 5];
        for _ in 0..500 {
            let i = rng.below(5);
            assert!(i < 5);
            seen[i] = true;
        }
        assert!(seen.iter().all(|s| *s), "{seen:?}");
    }

    // ── the two modes are two questions ──────────────────────────────────────

    #[test]
    fn a_reshuffle_reports_no_endpoint_dispersion_but_still_measures_the_path() {
        let s = simulate(&series(), &req(Mode::Reshuffle, None));
        assert!(s.endpoint_dispersion.is_none());
        let p50 = f(&s.max_drawdown, &["p50"]);
        let p99 = f(&s.max_drawdown, &["p99"]);
        assert!(p50 > 0.0, "p50 drawdown was {p50}");
        assert!(
            p99 > p50,
            "reordering must spread the drawdown: p50 {p50}, p99 {p99}"
        );
    }

    #[test]
    fn a_bootstrap_endpoint_actually_moves() {
        let s = simulate(&series(), &req(Mode::Bootstrap, None));
        let d = s.endpoint_dispersion.expect("bootstrap reports dispersion");
        assert!(
            f(&d, &["sd"]) > 0.0,
            "a resampled endpoint over a varying series must vary: {d}"
        );
        assert!(f(&d, &["p95"]) > f(&d, &["p5"]), "{d}");
    }

    /// The acceptance criterion, read literally: a reshuffle result cannot be
    /// reported as a capital requirement or a worst case.
    #[test]
    fn a_reshuffle_result_refuses_the_worst_case_label_and_reports_no_dispersion() {
        let out = resample_report(&rows(&series(), 1.0), &req(Mode::Reshuffle, None));
        assert_eq!(out.get("mode").and_then(Value::as_str), Some("reshuffle"));
        assert_eq!(out.get("endpoint_dispersion"), Some(&Value::Null));
        let refusal = out
            .get("not_a_worst_case")
            .and_then(Value::as_str)
            .expect("a reshuffle result says what it is not");
        assert!(refusal.contains("NOT a capital requirement"), "{refusal}");
        assert!(
            out.get("endpoint_dispersion_absent_because").is_some(),
            "{out}"
        );
    }

    #[test]
    fn a_bootstrap_result_carries_dispersion_and_no_worst_case_disclaimer() {
        let out = resample_report(&rows(&series(), 1.0), &req(Mode::Bootstrap, None));
        assert_eq!(out.get("mode").and_then(Value::as_str), Some("bootstrap"));
        assert!(out.get("not_a_worst_case").is_none(), "{out}");
        assert!(f(&out, &["endpoint_dispersion", "sd"]) > 0.0, "{out}");
    }

    #[test]
    fn every_result_names_the_mode_it_ran() {
        for mode in [Mode::Reshuffle, Mode::Bootstrap] {
            let out = resample_report(&rows(&series(), 1.0), &req(mode, None));
            assert_eq!(out.get("mode").and_then(Value::as_str), Some(mode.as_str()));
            assert!(
                out.get("measures")
                    .and_then(Value::as_str)
                    .is_some_and(|m| m.len() > 40),
                "{out}"
            );
        }
    }

    #[test]
    fn ruin_is_certain_below_the_deepest_trough_and_impossible_above_it() {
        // Every trade is a loss, so every ordering reaches -30 and no ordering
        // goes past it. Both ends are exact rather than nearly exact.
        let s = simulate(&all_losses(), &req(Mode::Reshuffle, Some(vec![0.5, 100.0])));
        let r = s.ruin.expect("levels were supplied");
        let arr = r.as_array().expect("array");
        assert_eq!(arr.len(), 2, "{r}");
        assert!(
            (f(&arr[0], &["prob"]) - 1.0).abs() < 1e-9,
            "capital 0.5 is always ruined: {r}"
        );
        assert!(
            f(&arr[1], &["prob"]).abs() < 1e-9,
            "capital 100 is never ruined: {r}"
        );
    }

    #[test]
    fn ruin_falls_as_capital_rises_and_stops_at_the_total_of_the_losses() {
        // `series()` loses 210 in total, so 300 of capital cannot be ruined at
        // any ordering — while a capital of 1 is ruined by any ordering that
        // opens with a loss, which is a third of them before counting the rest.
        let s = simulate(
            &series(),
            &req(Mode::Reshuffle, Some(vec![1.0, 50.0, 300.0])),
        );
        let r = s.ruin.expect("levels were supplied");
        let arr = r.as_array().expect("array");
        let p: Vec<f64> = arr.iter().map(|row| f(row, &["prob"])).collect();
        assert!(
            p[0] > 0.0 && p[0] < 1.0,
            "a mixed series is neither certain nor safe: {r}"
        );
        assert!(p[0] >= p[1], "ruin cannot rise with capital: {r}");
        assert!(p[1] >= p[2], "ruin cannot rise with capital: {r}");
        assert!(
            p[2].abs() < 1e-9,
            "capital above the total of the losses is never ruined: {r}"
        );
    }

    #[test]
    fn no_capital_levels_means_no_ruin_number_and_a_reason() {
        let out = resample_report(&rows(&series(), 1.0), &req(Mode::Reshuffle, None));
        assert_eq!(out.get("ruin_prob_by_capital"), Some(&Value::Null));
        assert!(out.get("ruin_prob_absent_because").is_some(), "{out}");
    }

    #[test]
    fn too_few_trades_is_refused_rather_than_answered() {
        let out = resample_report(&rows(&[1.0, 2.0, -1.0], 0.1), &req(Mode::Bootstrap, None));
        assert_eq!(
            out.get("error").and_then(Value::as_str),
            Some("too_few_trades")
        );
        assert_eq!(
            out.get("need").and_then(Value::as_u64),
            Some(MIN_TRADES as u64)
        );
        assert!(out.get("max_drawdown_pctiles").is_none(), "{out}");
    }

    // ── parsing ──────────────────────────────────────────────────────────────

    #[test]
    fn a_missing_mode_is_refused_and_the_refusal_explains_both_modes() {
        let e = parse_resample(&json!({ "run_id": "r1" })).expect_err("mode is required");
        assert_eq!(
            e.get("error").and_then(Value::as_str),
            Some("mode_required")
        );
        let why = e.get("why").and_then(Value::as_str).expect("why");
        assert!(why.contains("path risk"), "{why}");
        assert!(why.contains("estimation uncertainty"), "{why}");
    }

    #[test]
    fn block_mode_is_refused_by_name_and_names_the_symbol_it_would_need() {
        let e = parse_resample(&json!({ "run_id": "r1", "mode": "block" }))
            .expect_err("block is not offered");
        assert_eq!(e.get("error").and_then(Value::as_str), Some("unknown_mode"));
        let note = e
            .get("block_not_offered")
            .and_then(Value::as_str)
            .expect("note");
        assert!(note.contains("politis_white_block"), "{note}");
    }

    #[test]
    fn a_path_count_below_the_floor_is_refused() {
        let e = parse_resample(&json!({ "run_id": "r1", "mode": "bootstrap", "n_paths": 10 }))
            .expect_err("below the floor");
        assert_eq!(
            e.get("error").and_then(Value::as_str),
            Some("n_paths_out_of_range")
        );
    }

    #[test]
    fn the_defaults_are_the_ones_the_spec_names() {
        let r = parse_resample(&json!({ "run_id": "r1", "mode": "reshuffle" })).expect("parses");
        assert_eq!(r.n_paths, 5_000);
        assert_eq!(r.seed, 0);
        assert_eq!(r.capital_levels, None);
    }

    #[test]
    fn non_positive_capital_levels_are_dropped_rather_than_reported_as_certain_ruin() {
        let r = parse_resample(&json!({
            "run_id": "r1", "mode": "reshuffle", "capital_levels": [0.0, -5.0, 250.0]
        }))
        .expect("parses");
        assert_eq!(r.capital_levels, Some(vec![250.0]));
    }

    // ── reading the wire ─────────────────────────────────────────────────────

    #[test]
    fn a_decimal_is_read_whether_it_arrives_as_a_string_or_a_number() {
        let read = read_trades(&json!({ "trades": [
            { "pnl": "1.5", "costs_paid": "0.25", "entry_price": "10", "exit_price": "11", "qty": "2" },
            { "pnl": 1.5, "costs_paid": 0.25, "entry_price": 10, "exit_price": 11, "qty": 2 }
        ]}));
        assert_eq!(read.len(), 2);
        assert_eq!(
            read[0], read[1],
            "the string and number forms must read the same"
        );
        assert_eq!(read[0].costs_paid, Some(0.25));
        assert_eq!(read[0].notional, Some(42.0));
    }

    #[test]
    fn a_trade_with_no_costs_field_reads_as_unknown_not_as_zero() {
        let read = read_trades(&json!({ "trades": [{ "pnl": 1.0 }] }));
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].costs_paid, None);
        assert_eq!(read[0].notional, None);
    }

    // ── the cost curve ───────────────────────────────────────────────────────

    fn shock(run: &str) -> ShockRequest {
        ShockRequest {
            run_id: run.into(),
            multipliers: DEFAULT_MULTIPLIERS.to_vec(),
            live_cost_bps: None,
        }
    }

    #[test]
    fn the_breakeven_multiple_is_where_the_net_reaches_zero() {
        // Forty trades, each +10 net after 5 of costs: gross 600, costs 200,
        // net 400. Break-even is 600 / 200 = 3.
        let trades = rows(&vec![10.0; 40], 5.0);
        let out = shock_report(&trades, &shock("r1"));
        assert!(
            (f(&out, &["breakeven_multiplier"]) - 3.0).abs() < 1e-9,
            "{out}"
        );
        let curve = out.get("curve").and_then(Value::as_array).expect("curve");
        assert_eq!(curve.len(), DEFAULT_MULTIPLIERS.len());
        // The 3x point is exactly zero, so it is not profitable.
        let at_3 = curve
            .iter()
            .find(|p| f(p, &["multiplier"]) == 3.0)
            .expect("3x is on the grid");
        assert!(f(at_3, &["net_pnl"]).abs() < 1e-9, "{at_3}");
        assert_eq!(at_3.get("profitable").and_then(Value::as_bool), Some(false));
        let at_1 = curve
            .iter()
            .find(|p| f(p, &["multiplier"]) == 1.0)
            .expect("1x is on the grid");
        assert!((f(at_1, &["net_pnl"]) - 400.0).abs() < 1e-9, "{at_1}");
    }

    #[test]
    fn a_run_already_losing_money_says_so_and_reports_a_multiple_below_one() {
        let trades = rows(&vec![-1.0; 40], 5.0);
        let out = shock_report(&trades, &shock("r1"));
        assert_eq!(
            out.get("already_unprofitable").and_then(Value::as_bool),
            Some(true)
        );
        assert!(f(&out, &["breakeven_multiplier"]) < 1.0, "{out}");
    }

    #[test]
    fn a_run_with_no_recorded_costs_is_refused_rather_than_given_an_infinite_margin() {
        let trades = rows(&vec![10.0; 40], 0.0);
        let out = shock_report(&trades, &shock("r1"));
        assert_eq!(
            out.get("error").and_then(Value::as_str),
            Some("no_costs_charged")
        );
        assert!(out.get("breakeven_multiplier").is_none(), "{out}");
    }

    #[test]
    fn one_trade_missing_its_costs_refuses_the_whole_curve() {
        let mut trades = rows(&vec![10.0; 40], 5.0);
        trades[7].costs_paid = None;
        let out = shock_report(&trades, &shock("r1"));
        assert_eq!(
            out.get("error").and_then(Value::as_str),
            Some("costs_not_recorded")
        );
        assert_eq!(
            out.get("trades_missing_costs_paid").and_then(Value::as_u64),
            Some(1)
        );
        assert!(out.get("curve").is_none(), "{out}");
    }

    #[test]
    fn the_grid_says_where_its_one_times_point_came_from() {
        let out = shock_report(&rows(&vec![10.0; 40], 5.0), &shock("r1"));
        let origin = out
            .get("grid_origin")
            .and_then(Value::as_str)
            .expect("grid_origin");
        assert!(origin.contains("costs_paid"), "{origin}");
        assert!(
            (f(&out, &["baseline", "total_costs"]) - 200.0).abs() < 1e-9,
            "{out}"
        );
        // 200 of costs against 40 trades of 1,000 notional each — 40,000 in all —
        // is 50 basis points.
        assert!(
            (f(&out, &["baseline_cost_bps_of_notional"]) - 50.0).abs() < 1e-9,
            "{out}"
        );
    }

    #[test]
    fn an_unrecorded_notional_leaves_the_basis_point_view_absent_not_zero() {
        let mut trades = rows(&vec![10.0; 40], 5.0);
        trades[0].notional = None;
        let out = shock_report(&trades, &shock("r1"));
        assert_eq!(out.get("baseline_cost_bps_of_notional"), Some(&Value::Null));
        assert!(
            out.get("baseline_cost_bps_absent_because").is_some(),
            "{out}"
        );
    }

    #[test]
    fn a_measured_live_cost_is_placed_on_the_curve_and_an_absent_one_is_not_guessed() {
        let trades = rows(&vec![10.0; 40], 5.0);
        let mut with_live = shock("r1");
        // The baseline is 50 bps and break-even is 3x, so a measured 100 bps is
        // 2x with a margin of 1.
        with_live.live_cost_bps = Some(100.0);
        let out = shock_report(&trades, &with_live);
        assert!(
            (f(&out, &["live_cost", "implied_multiplier"]) - 2.0).abs() < 1e-9,
            "{out}"
        );
        assert!(
            (f(&out, &["live_cost", "safety_margin"]) - 1.0).abs() < 1e-9,
            "{out}"
        );
        assert_eq!(
            out.get("live_cost")
                .and_then(|l| l.get("survives_live_costs"))
                .and_then(Value::as_bool),
            Some(true)
        );

        let without = shock_report(&trades, &shock("r1"));
        assert_eq!(without.get("live_cost"), Some(&Value::Null));
        assert!(
            without.get("live_cost_absent_because").is_some(),
            "{without}"
        );
    }

    #[test]
    fn a_negative_multiplier_is_refused() {
        let e = parse_shock(&json!({ "run_id": "r1", "multipliers": [1.0, -2.0] }))
            .expect_err("negative costs are not a shock");
        assert_eq!(
            e.get("error").and_then(Value::as_str),
            Some("bad_multipliers")
        );
    }

    #[test]
    fn the_default_grid_is_the_one_the_spec_names() {
        let r = parse_shock(&json!({ "run_id": "r1" })).expect("parses");
        assert_eq!(r.multipliers, vec![1.0, 1.5, 2.0, 3.0, 5.0]);
        assert_eq!(r.live_cost_bps, None);
    }

    // ── the catalogue declaration ────────────────────────────────────────────

    #[test]
    fn both_tools_are_declared_and_neither_spends_a_trial() {
        let specs = specs();
        assert_eq!(specs.len(), 2);
        for s in &specs {
            assert_eq!(s.namespace, "falsification", "{}", s.name);
            assert_eq!(s.risk, Risk::Read, "{}", s.name);
            assert_eq!(
                s.spends_trial,
                TrialKind::None,
                "`{}` dispatches nothing — it re-reads a run whose trial was charged when it \
                 ran. A trial here would inflate N_eff against work nobody did.",
                s.name
            );
            assert_eq!(s.envelope, crate::spec::Envelope::None, "{}", s.name);
        }
    }

    /// The whole catalogue, not this module's list: a `specs()` nobody calls is
    /// two tools that exist and are invisible.
    #[test]
    fn both_tools_reach_the_catalogue() {
        for name in ["trade_reshuffle_bootstrap", "cost_shock_curve"] {
            let s = crate::spec::find(name)
                .unwrap_or_else(|| panic!("`{name}` is not in spec::catalog()"));
            assert_eq!(s.spends_trial, TrialKind::None, "{name}");
            assert_eq!(s.namespace, "falsification", "{name}");
        }
    }

    /// The grid is multiples of a measured cost, and this module holds no
    /// basis-point figure it could quietly fall back to.
    ///
    /// Read from the source text because the failure is a *future* edit: a
    /// `const DEFAULT_SPREAD_BPS` added as a convenience turns a refusal — "this
    /// run recorded no costs" — into a plausible curve computed from a number
    /// nobody measured.
    #[test]
    fn no_default_cost_in_basis_points_is_written_into_this_module() {
        const SRC: &str = include_str!("path_risk.rs");
        let shipping = SRC
            .split_once("mod tests {")
            .expect("the test module is the last thing in this file")
            .0;
        for line in shipping.lines() {
            let t = line.trim_start();
            if t.starts_with("//") {
                continue;
            }
            assert!(
                !(t.contains("const ") && t.to_lowercase().contains("bps")),
                "a basis-point constant appeared in this module:\n  {t}\nThe 1x point of the \
                 cost grid is the run's own charged costs; a default spread would let the tool \
                 answer where it should refuse."
            );
        }
        assert!(
            shipping.contains("DEFAULT_MULTIPLIERS"),
            "the scan did not find the module it was meant to read"
        );
    }
}
