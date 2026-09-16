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
}

#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    #[error("{0}")]
    Feature(#[from] features::FeatureError),
    #[error("the window has {have} bars and a preview needs at least {need}")]
    TooShort { have: usize, need: usize },
    #[error("{0}")]
    Requirements(String),
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
    let names: Vec<String> = requirements.features.iter().map(|f| f.name.clone()).collect();
    let indicators: Vec<(String, std::sync::Arc<dyn features::Feature>)> = names
        .iter()
        .map(|n| Ok((n.clone(), features::runtime::feature(n)?)))
        .collect::<Result<_, features::FeatureError>>()?;

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
        rows.push(features::FeatureRow {
            ts_ns: bar.ts_ns,
            open: bar.open.to_f64().unwrap_or(0.0),
            high: bar.high.to_f64().unwrap_or(0.0),
            low: bar.low.to_f64().unwrap_or(0.0),
            close,
            volume: bar.volume.to_f64().unwrap_or(0.0),
        });
        let decision = rows.len() - 1;

        for (name, feature) in &indicators {
            match features::runtime::value_at(feature.as_ref(), &rows, decision) {
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
        ))
    };

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
    })
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
) -> (&'static str, String) {
    if (warmup as usize) >= n_bars {
        return ("warmup_exceeds_window", format!(
            "the longest feature needs {warmup} bars of warm-up and the window has {n_bars}. \
             No signal could fire here whatever the conditions say — widen the window."
        ));
    }
    if evaluated == 0 {
        return ("warmup_exceeds_window", format!(
            "no bar had a value for every feature, so the conditions were never evaluated on a \
             complete input set. Warm-up is {warmup} bars over a {n_bars}-bar window."
        ));
    }
    if let Some(dead) = features.iter().find(|f| f.bars_with_value == 0) {
        return ("warmup_exceeds_window", format!(
            "`{}` produced no value on any bar, so any condition naming it is false everywhere. \
             It needs {} bars of warm-up.",
            dead.name, dead.warmup_bars
        ));
    }
    if signals.is_empty() {
        return ("never_fires", format!(
            "the conditions were evaluated on {evaluated} bars and no signal was ever true. The \
             inputs are fine; the thresholds are not met on this window."
        ));
    }
    // The signals fired; nothing was listening. Worth naming separately because
    // every other reason here sends the author to the `nodes` block, and this
    // one is the only one whose fix is in `actions`.
    if let Some(flag) = signals.iter().find(|s| s.fires > 0) {
        if !ordering.contains(flag.signal.as_str()) {
            return ("signal_has_no_action", format!(
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
            ));
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
