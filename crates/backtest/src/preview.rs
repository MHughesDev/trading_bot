//! A dry run: what would this strategy do, without doing it (AGENT-011 §6; S-5.4).
//!
//! Step 1 of the research protocol becomes write → **preview** → adjust →
//! create. A strategy that never fires, or fires on every bar, is caught before
//! a counted trial rather than after one.
//!
//! # It shares the simulator's engines rather than resembling them
//!
//! The acceptance is that `sample_events` match the fills of a later backtest on
//! the same window. That is only achievable if the preview evaluates the *same*
//! way, so it uses the same two pieces `sim::build_handler` uses —
//! `features::runtime` for every indicator and
//! `strategy_runtime::evaluate_signals` for the node graph — and reproduces the
//! one piece of semantics that is easy to get wrong:
//!
//! **the rising edge.** A signal fires when it becomes true, not while it is
//! true. The simulator emits an order for each signal in
//! `fired.difference(&active_signals)` and then replaces the active set. A
//! preview that counted every bar where a condition held would report a hundred
//! entries where the backtest takes three, and the tool would be worse than
//! nothing — a model would learn to distrust it.
//!
//! # What it does not do
//!
//! No fills, no position, no P&L, no exits beyond naming the declared policy.
//! Those need the order book and the exit tracker, and a preview that
//! approximated them would be a second simulator with different answers. The
//! question this answers is narrower and is the one worth asking first: does
//! this strategy do anything at all, and how often.
//!
//! **No trial and no ledger row.** That is the point, and it is also the risk:
//! see `crates/api`'s preview route for the holdout refusal, which is what stops
//! "no trial" from quietly becoming "no rules".

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use domain::payloads::bar::{BarPayload, Timeframe};
use domain::strategy_def::StrategyDefinition;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};

use crate::perturb::{AppliedPerturbation, Perturbation, PerturbationSet};
use crate::store::LoadedBar;

/// One bar on which a signal became true.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreviewEvent {
    pub signal: String,
    pub at: DateTime<Utc>,
    /// Index into the window, so a caller can see clustering without timestamps.
    pub bar: usize,
    pub close: f64,
    /// The feature values at the moment it fired — the reason, not just the fact.
    pub features: HashMap<String, f64>,
}

/// How often one signal fired.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalStat {
    pub signal: String,
    pub fires: usize,
    pub first_at: Option<DateTime<Utc>>,
    pub last_at: Option<DateTime<Utc>>,
    /// Bars on which the condition held, whether or not it was a rising edge.
    /// The gap between this and `fires` is how long the condition stays true —
    /// a signal true on 400 of 500 bars that fires twice is a regime flag, not
    /// an entry trigger, and the two numbers say so where one would not.
    pub bars_true: usize,
}

/// What a feature did over the window.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeatureStat {
    pub name: String,
    pub warmup_bars: u32,
    /// Bars on which it produced a value at all.
    pub bars_with_value: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalPreview {
    pub n_bars: usize,
    /// The longest warm-up among the strategy's features: bars that can produce
    /// no signal because an input has no value yet.
    pub warmup_bars: u32,
    pub bars_evaluated: usize,
    pub entries: usize,
    pub entries_per_month: Option<f64>,
    pub signals: Vec<SignalStat>,
    pub features: Vec<FeatureStat>,
    pub sample_events: Vec<PreviewEvent>,
    /// Set when the strategy produced no entry, saying which of the several
    /// possible reasons it was. A bare zero sends the author to change the
    /// wrong thing.
    pub never_fires_because: Option<String>,
    /// The same diagnosis as a stable code, for a caller that wants to branch
    /// rather than read (AGENT-011 §5). The prose says which of the reasons it
    /// was; this is what a retry policy, a renderer or a test can act on, and
    /// what `strategy_validator::teach` attaches a fix to.
    pub never_fires_code: Option<&'static str>,
    pub exit_policy: Option<String>,
    /// Every input perturbation this run was computed under (S-3.10). Empty on
    /// an ordinary preview, and **always serialized** — a caller that cannot see
    /// the key is talking to an older server, which is a different fact from a
    /// run that was not perturbed.
    ///
    /// Carried on the result and not only in the request because the comparison
    /// this feeds is between two previews, and the reader has to be able to tell
    /// which one it is holding. A perturbed run that looked ordinary would let a
    /// diff of two unperturbed runs report "no lookahead detected".
    #[serde(default)]
    pub perturbations: Vec<crate::perturb::AppliedPerturbation>,
}

#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    #[error("{0}")]
    Feature(#[from] features::FeatureError),
    #[error("the window has {have} bars and a preview needs at least {need}")]
    TooShort { have: usize, need: usize },
    #[error("{0}")]
    Requirements(String),
    #[error("{0}")]
    Perturbation(#[from] crate::perturb::PerturbationError),
}

/// The fewest bars worth previewing.
///
/// Not a round number for its own sake: below this the warm-up dominates every
/// answer, and "it never fired" would be about the window rather than about the
/// strategy.
const MIN_BARS: usize = 30;

/// At most this many events come back. The first ones, not a sample: a caller
/// checking whether the strategy fires at all wants the earliest firings, and
/// the count is reported separately anyway.
const MAX_EVENTS: usize = 20;

/// Walk the window and report what would have fired.
///
/// # Errors
/// [`PreviewError::Feature`] for a name the engine cannot resolve — which is the
/// same error the backtest would give, at the same point, for the same reason;
/// [`PreviewError::TooShort`] for a window too short to say anything.
pub fn preview_signals(
    definition: &StrategyDefinition,
    bars: &[LoadedBar],
    timeframe: Timeframe,
) -> Result<SignalPreview, PreviewError> {
    preview_signals_perturbed(definition, bars, timeframe, &PerturbationSet::default())
}

/// The same walk, over deliberately corrupted feature columns (S-3.10).
///
/// This is the runtime half of `future_perturbation_test`: run once clean, run
/// again with one column shifted forward, and diff. A strategy whose answer does
/// not change when a column is handed values from bars it had not seen was
/// already reading those bars.
///
/// # What the perturbation does and does not touch
///
/// Only the columns the set names. Every other feature is evaluated through the
/// ordinary incremental path — the same `value_at` over the same growing window —
/// so the unperturbed half of a perturbed run is byte-identical to a clean run,
/// and a divergence can only have come from the column that was named. The
/// perturbed columns are produced by `features::runtime::backfill_column` over
/// the whole window and then indexed with the shift; INV-14
/// (`features_versioned::backfill_and_incremental_serving_agree`) is what makes
/// those two paths substitutable, and it is asserted in `crates/features`
/// rather than assumed here.
///
/// `crates/features` is **unchanged** by this. See the module docs in
/// [`crate::perturb`] for why a `shift` parameter does not belong inside
/// `backfill_column`.
///
/// # Errors
/// Everything [`preview_signals`] returns, plus
/// [`PreviewError::Perturbation`] for a request whose no-divergence result would
/// have been meaningless — an empty set, a zero-bar shift, a column the strategy
/// does not read, or a shift wider than the window.
pub fn preview_signals_perturbed(
    definition: &StrategyDefinition,
    bars: &[LoadedBar],
    timeframe: Timeframe,
    perturbations: &PerturbationSet,
) -> Result<SignalPreview, PreviewError> {
    if bars.len() < MIN_BARS {
        return Err(PreviewError::TooShort {
            have: bars.len(),
            need: MIN_BARS,
        });
    }

    // The feature list comes from `derive_requirements`, which is where the
    // executor gets it (`sim_executor.rs:305`). Reading the definition's inputs
    // directly here would be a second derivation, and the two would disagree the
    // first time the rules for deriving a feature list changed.
    let requirements = crate::requirements::derive_requirements(definition, timeframe)
        .map_err(|e| PreviewError::Requirements(e.to_string()))?;
    let names: Vec<String> = requirements
        .features
        .iter()
        .map(|f| f.name.clone())
        .collect();
    let indicators: Vec<(String, std::sync::Arc<dyn features::Feature>)> = names
        .iter()
        .map(|n| Ok((n.clone(), features::runtime::feature(n)?)))
        .collect::<Result<_, features::FeatureError>>()?;

    // Validated against the columns this strategy actually reads, not against
    // the feature catalogue: a perturbation of a column nothing reads cannot
    // change the answer, and its no-divergence result would be evidence about
    // nothing. Validation is here, after `derive_requirements`, because that is
    // the only place the real column list exists.
    let perturbed: HashMap<String, Vec<Option<f64>>> = if perturbations.is_empty() {
        HashMap::new()
    } else {
        let set = PerturbationSet::validate(perturbations.items().to_vec(), &names, bars.len())?;
        let all_rows: Vec<features::FeatureRow> = bars.iter().map(feature_row).collect();
        indicators
            .iter()
            .filter_map(|(name, f)| {
                let p = set.get(name)?;
                let column = features::runtime::backfill_column(f.as_ref(), &all_rows);
                Some((name.clone(), crate::perturb::apply(p, &column)))
            })
            .collect()
    };

    let warmup = indicators
        .iter()
        .map(|(_, f)| f.def().lookback_bars)
        .max()
        .unwrap_or(0);
    let keep = (warmup as usize).max(1);

    let mut rows: Vec<features::FeatureRow> = Vec::with_capacity(keep * 2);
    let mut feature_values: HashMap<String, f64> = HashMap::new();
    let mut bar_map: HashMap<Timeframe, BarPayload> = HashMap::new();
    let mut active: HashSet<String> = HashSet::new();

    // Which signals the simulator would actually place an order on.
    //
    // `sim.rs:1109` walks the rising edges and then looks for an order spec
    // whose `on_signal` matches; a signal with no matching action produces no
    // command. Counting every rising edge as an entry would therefore overstate
    // the count for any definition carrying a signal it does not trade on —
    // a regime flag, or an exit signal named in `ExitRule::OnSignal`. The
    // signal stats below still report every signal, because a caller wants to
    // see the flag firing; only `entries` is restricted.
    let ordering: HashSet<&str> = definition
        .actions
        .iter()
        .map(|a| match a.kind {
            domain::strategy_def::actions::ActionKind::PlaceOrder { .. } => a.on_signal.as_str(),
        })
        .collect();

    let mut fires: HashMap<String, usize> = HashMap::new();
    let mut bars_true: HashMap<String, usize> = HashMap::new();
    let mut first_at: HashMap<String, DateTime<Utc>> = HashMap::new();
    let mut last_at: HashMap<String, DateTime<Utc>> = HashMap::new();
    let mut with_value: HashMap<String, usize> = HashMap::new();
    let mut events: Vec<PreviewEvent> = Vec::new();
    let mut evaluated = 0usize;
    let mut entries = 0usize;

    for (i, bar) in bars.iter().enumerate() {
        if rows.len() >= keep * 2 {
            rows.drain(..rows.len() + 1 - keep);
        }
        let close = bar.close.to_f64().unwrap_or(0.0);
        rows.push(feature_row(bar));
        let decision = rows.len() - 1;

        for (name, feature) in &indicators {
            // A perturbed column is read out of the pre-computed column; every
            // other feature takes the ordinary incremental path, so the
            // unperturbed half of a perturbed run is the clean run.
            let value = match perturbed.get(name) {
                Some(column) => column.get(i).copied().flatten(),
                None => features::runtime::value_at(feature.as_ref(), &rows, decision),
            };
            match value {
                Some(v) => {
                    feature_values.insert(name.clone(), v);
                    *with_value.entry(name.clone()).or_default() += 1;
                }
                None => {
                    feature_values.remove(name);
                }
            }
        }

        // The grammar reads bar fields from the 1m lane specifically, so the
        // payload is registered under both, exactly as the simulator does.
        let payload = BarPayload::new(
            timeframe,
            domain::money::Price::from_decimal(bar.open),
            domain::money::Price::from_decimal(bar.high),
            domain::money::Price::from_decimal(bar.low),
            domain::money::Price::from_decimal(bar.close),
            domain::money::Size::from_decimal(bar.volume),
            0,
        );
        bar_map.insert(timeframe, payload.clone());
        bar_map.insert(Timeframe::Minutes1, payload);

        // A bar counts as evaluated once every feature has a value; before that
        // the graph is being asked about inputs that do not exist yet, and
        // counting those bars would understate the firing rate.
        if feature_values.len() == indicators.len() {
            evaluated += 1;
        }

        let true_now: HashSet<String> =
            strategy_runtime::evaluate_signals(&definition.nodes, &feature_values, &bar_map)
                .into_iter()
                .collect();

        for signal in &true_now {
            *bars_true.entry(signal.clone()).or_default() += 1;
        }
        // The rising edge — see the module docs. This is the line that makes a
        // preview agree with a backtest.
        for signal in true_now.difference(&active) {
            if ordering.contains(signal.as_str()) {
                entries += 1;
            }
            *fires.entry(signal.clone()).or_default() += 1;
            let at = ts_to_utc(bar.ts_ns);
            first_at.entry(signal.clone()).or_insert(at);
            last_at.insert(signal.clone(), at);
            if events.len() < MAX_EVENTS {
                events.push(PreviewEvent {
                    signal: signal.clone(),
                    at,
                    bar: i,
                    close,
                    features: feature_values.clone(),
                });
            }
        }
        active = true_now;
    }

    let mut signals: Vec<SignalStat> = bars_true
        .keys()
        .chain(fires.keys())
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|name| SignalStat {
            signal: name.clone(),
            fires: fires.get(name).copied().unwrap_or(0),
            first_at: first_at.get(name).copied(),
            last_at: last_at.get(name).copied(),
            bars_true: bars_true.get(name).copied().unwrap_or(0),
        })
        .collect();
    signals.sort_by(|a, b| b.fires.cmp(&a.fires).then(a.signal.cmp(&b.signal)));

    let mut feature_stats: Vec<FeatureStat> = indicators
        .iter()
        .map(|(name, f)| FeatureStat {
            name: name.clone(),
            warmup_bars: f.def().lookback_bars,
            bars_with_value: with_value.get(name).copied().unwrap_or(0),
        })
        .collect();
    feature_stats.sort_by(|a, b| a.name.cmp(&b.name));

    let span_months = span_months(bars);
    let entries_per_month = span_months.filter(|m| *m > 0.0).map(|m| entries as f64 / m);

    let never = if entries > 0 {
        None
    } else {
        Some(diagnose(
            bars.len(),
            warmup,
            evaluated,
            &feature_stats,
            &signals,
            &ordering,
            perturbations,
        ))
    };

    let applied: Vec<AppliedPerturbation> = perturbations
        .items()
        .iter()
        .map(|p| {
            let (shift_bars, perturbation_pct, seed) = match p {
                Perturbation::ShiftColumn { shift_bars, .. } => (Some(*shift_bars), None, None),
                Perturbation::MaskColumn { .. } => (None, None, None),
                Perturbation::JitterColumn {
                    perturbation_pct,
                    seed,
                    ..
                } => (None, Some(*perturbation_pct), Some(*seed)),
            };
            AppliedPerturbation {
                mode: p.mode(),
                column: p.column().to_string(),
                shift_bars,
                perturbation_pct,
                seed,
                bars_with_value: perturbed
                    .get(p.column())
                    .map_or(0, |c| c.iter().filter(|v| v.is_some()).count()),
            }
        })
        .collect();

    Ok(SignalPreview {
        n_bars: bars.len(),
        warmup_bars: warmup,
        bars_evaluated: evaluated,
        entries,
        entries_per_month,
        signals,
        features: feature_stats,
        sample_events: events,
        never_fires_because: never.as_ref().map(|(_, why)| why.clone()),
        never_fires_code: never.as_ref().map(|(code, _)| *code),
        exit_policy: definition.exit_policy().map(|p| format!("{p:?}")),
        perturbations: applied,
    })
}

/// One bar as the feature engine reads it.
fn feature_row(bar: &LoadedBar) -> features::FeatureRow {
    features::FeatureRow {
        ts_ns: bar.ts_ns,
        open: bar.open.to_f64().unwrap_or(0.0),
        high: bar.high.to_f64().unwrap_or(0.0),
        low: bar.low.to_f64().unwrap_or(0.0),
        close: bar.close.to_f64().unwrap_or(0.0),
        volume: bar.volume.to_f64().unwrap_or(0.0),
    }
}

/// Why nothing fired — named, because a bare zero sends the author to change the
/// wrong thing.
///
/// The order is the order of blame: a window too short for the warm-up is not a
/// strategy problem, a feature with no values is not a condition problem, and
/// only when both of those are fine is the condition itself the answer.
fn diagnose(
    n_bars: usize,
    warmup: u32,
    evaluated: usize,
    features: &[FeatureStat],
    signals: &[SignalStat],
    ordering: &HashSet<&str>,
    perturbations: &PerturbationSet,
) -> (&'static str, String) {
    // Blame the perturbation before anything else, because it is the one cause
    // the caller introduced on purpose. A masked column leaves every bar without
    // a complete input set, which walks straight into the warm-up branch below
    // and reports a window problem — sending a reader to widen a window that was
    // never the reason, and, worse, letting a perturbed run that fired nothing be
    // read as a run the perturbation did not reach.
    if !perturbations.is_empty() {
        if let Some(dead) = features.iter().find(|f| f.bars_with_value == 0) {
            if perturbations.touches(&dead.name) {
                return (
                    "perturbed_column_has_no_value",
                    format!(
                        "`{}` was perturbed and has a value on no bar, so any condition naming it \
                         is false everywhere and nothing could fire. This is the perturbation, not \
                         the window or the thresholds — a divergence from the unperturbed run here \
                         says only that the column was removed.",
                        dead.name
                    ),
                );
            }
        }
    }
    if (warmup as usize) >= n_bars {
        return (
            "warmup_exceeds_window",
            format!(
                "the longest feature needs {warmup} bars of warm-up and the window has {n_bars}. \
             No signal could fire here whatever the conditions say — widen the window."
            ),
        );
    }
    if evaluated == 0 {
        return (
            "warmup_exceeds_window",
            format!(
            "no bar had a value for every feature, so the conditions were never evaluated on a \
             complete input set. Warm-up is {warmup} bars over a {n_bars}-bar window."
        ),
        );
    }
    if let Some(dead) = features.iter().find(|f| f.bars_with_value == 0) {
        return (
            "warmup_exceeds_window",
            format!(
            "`{}` produced no value on any bar, so any condition naming it is false everywhere. \
             It needs {} bars of warm-up.",
            dead.name, dead.warmup_bars
        ),
        );
    }
    if signals.is_empty() {
        return (
            "never_fires",
            format!(
            "the conditions were evaluated on {evaluated} bars and no signal was ever true. The \
             inputs are fine; the thresholds are not met on this window."
        ),
        );
    }
    // The signals fired; nothing was listening. Worth naming separately because
    // every other reason here sends the author to the `nodes` block, and this
    // one is the only one whose fix is in `actions`.
    if let Some(flag) = signals.iter().find(|s| s.fires > 0) {
        if !ordering.contains(flag.signal.as_str()) {
            return (
                "signal_has_no_action",
                format!(
                    "`{}` became true {} times, but no action names it — `actions` lists {}. The \
                 conditions are working; nothing is wired to trade them.",
                    flag.signal,
                    flag.fires,
                    if ordering.is_empty() {
                        "no signal at all".to_string()
                    } else {
                        let mut names: Vec<&str> = ordering.iter().copied().collect();
                        names.sort_unstable();
                        names.join(", ")
                    }
                ),
            );
        }
    }
    (
        "never_fires",
        format!(
            "signals were true on some bars but never became true — they were already true at the \
         first evaluated bar and stayed that way. An entry fires on the rising edge, so a \
         condition that is always true never fires. Evaluated on {evaluated} bars."
        ),
    )
}

fn ts_to_utc(ns: i64) -> DateTime<Utc> {
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let sub = ns.rem_euclid(1_000_000_000) as u32;
    DateTime::<Utc>::from_timestamp(ns.div_euclid(1_000_000_000), sub).unwrap_or_default()
}

/// Calendar months the window spans, for a rate a person can reason about.
fn span_months(bars: &[LoadedBar]) -> Option<f64> {
    let first = bars.first()?.ts_ns;
    let last = bars.last()?.ts_ns;
    let seconds = (last - first) as f64 / 1e9;
    (seconds > 0.0).then(|| seconds / (365.25 / 12.0 * 86_400.0))
}

// ── v3: a preview is a short backtest (Set U U-3.5) ──────────────────────────
//
// For a `tbot.strategy/3` graph there is no separate preview loop. A preview
// runs the backtest runner — the same engine, the same bars, the same merge,
// the same venue — over a short window and reports only what was decided:
// decisions, intents, holds and their reasons. No fills, no P&L and no trial,
// for the same reason as above: a preview that reported a return would be a
// backtest with the ledger switched off. Because it *is* the backtest's loop,
// its decisions match a backtest over the same window by construction, and a
// test holds it to that.

/// Which venue a v3 preview runs against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewVenue {
    /// The simulated venue, exactly as a backtest uses it: positions open and
    /// close, so every later decision sees the account a backtest would.
    #[default]
    Simulated,
    /// Orders are recorded and never executed; the account stays flat. Answers
    /// "when would it act?" for a strategy whose entries do not depend on its
    /// position.
    IntentOnly,
}

/// One decision that did something worth showing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviewDecision {
    pub at: DateTime<Utc>,
    pub tick: u64,
    /// `bar_close`, `fill`, `order_closed`.
    pub cause: String,
    /// (rule index, the intent it resolved to).
    pub intents: Vec<(usize, strategy_engine::planner::ResolvedIntent)>,
    pub actions: Vec<strategy_engine::planner::OrderAction>,
    /// (rule index, why it held).
    pub holds: Vec<(usize, Vec<String>)>,
    pub refusals: Vec<String>,
}

/// One rule across the window: when it could first have fired, and what
/// stopped it before then.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RulePreview {
    pub rule: usize,
    /// The first decision at which the rule fired, if it ever did.
    pub first_fired_at: Option<DateTime<Utc>>,
    pub fired: u64,
    pub held: u64,
    /// Outcomes before the first firing (or over the whole window if it never
    /// fired): `not_matched`, `shadowed`, `unknown:<reason>`, `held:<reason>`.
    pub before_first_fire: std::collections::BTreeMap<String, u64>,
}

/// What a v3 preview reports.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct V3Preview {
    pub venue: PreviewVenue,
    /// Bar-close decisions inside the window.
    pub decisions: u64,
    pub rules: Vec<RulePreview>,
    /// Every decision that placed an order, held, or was refused.
    pub notable: Vec<PreviewDecision>,
    /// The same digest a backtest's manifest carries over the same decisions.
    pub journal_digest: String,
}

fn reason_key(m: &domain::strategy_graph::MissingReason) -> String {
    serde_json::to_value(m)
        .ok()
        .and_then(|v| v.get("reason").and_then(|r| r.as_str()).map(str::to_string))
        .unwrap_or_else(|| "unknown".into())
}

fn cause_key(c: &strategy_engine::trace::Cause) -> &'static str {
    use strategy_engine::trace::Cause;
    match c {
        Cause::BarClose => "bar_close",
        Cause::Fill { .. } => "fill",
        Cause::OrderClosed { .. } => "order_closed",
        Cause::Schedule => "schedule",
    }
}

/// Summarise a run's decisions as a preview.
#[must_use]
pub fn summarise(
    venue: PreviewVenue,
    rule_count: usize,
    traces: &[strategy_engine::DecisionTrace],
) -> V3Preview {
    use sha2::Digest;
    use strategy_engine::decide::RuleOutcome;
    use strategy_engine::trace::Cause;

    let mut rules: Vec<RulePreview> = (0..rule_count)
        .map(|rule| RulePreview {
            rule,
            first_fired_at: None,
            fired: 0,
            held: 0,
            before_first_fire: std::collections::BTreeMap::new(),
        })
        .collect();
    let mut notable = Vec::new();
    let mut chain = sha2::Sha256::new();
    let mut decisions = 0u64;
    for t in traces {
        chain.update(t.digest().as_bytes());
        if t.cause == Cause::BarClose {
            decisions += 1;
        }
        let at = DateTime::from_timestamp_nanos(t.at_ns);
        let mut holds = Vec::new();
        for (&i, outcome) in &t.rules {
            let Some(r) = rules.get_mut(i) else { continue };
            let key = match outcome {
                RuleOutcome::Fired => {
                    r.fired += 1;
                    if r.first_fired_at.is_none() {
                        r.first_fired_at = Some(at);
                    }
                    continue;
                }
                RuleOutcome::Held { reasons } => {
                    r.held += 1;
                    holds.push((i, reasons.iter().map(ToString::to_string).collect()));
                    format!(
                        "held:{}",
                        reasons.first().map_or_else(|| "unknown".into(), reason_key)
                    )
                }
                RuleOutcome::NotMatched => "not_matched".into(),
                RuleOutcome::Shadowed => "shadowed".into(),
                RuleOutcome::Unknown { reason } => format!("unknown:{}", reason_key(reason)),
            };
            if r.first_fired_at.is_none() {
                *r.before_first_fire.entry(key).or_insert(0) += 1;
            }
        }
        if !t.intents.is_empty()
            || !t.actions.is_empty()
            || !holds.is_empty()
            || !t.refusals.is_empty()
        {
            notable.push(PreviewDecision {
                at,
                tick: t.tick,
                cause: cause_key(&t.cause).to_string(),
                intents: t.intents.clone(),
                actions: t.actions.clone(),
                holds,
                refusals: t.refusals.iter().map(|r| r.reason.clone()).collect(),
            });
        }
    }
    V3Preview {
        venue,
        decisions,
        rules,
        notable,
        journal_digest: hex::encode(chain.finalize()),
    }
}

/// Preview a v3 run: the backtest loop over its window, reporting decisions
/// only.
///
/// # Errors
/// As [`crate::runner::run`].
pub fn preview_v3(
    r: crate::runner::V3Run,
    venue: PreviewVenue,
    models: &mut dyn strategy_engine::env::ModelProvider,
) -> Result<(V3Preview, Vec<strategy_engine::DecisionTrace>), crate::runner::RunnerError> {
    let rule_count = r.plan.rules.len();
    let traces = match venue {
        PreviewVenue::Simulated => crate::runner::run(r, models)?.traces,
        PreviewVenue::IntentOnly => {
            let primary = crate::runner::primary_bars(&r)?;
            let mut v = crate::env::IntentOnly::new(r.spec.clone(), r.initial_balance);
            crate::runner::drive(
                &r.plan,
                &r.bars,
                &r.external,
                &primary,
                r.start_ns,
                &mut v,
                models,
                &mut strategy_engine::env::NoJournal,
            )?
        }
    };
    Ok((summarise(venue, rule_count, &traces), traces))
}
