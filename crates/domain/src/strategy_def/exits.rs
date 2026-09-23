//! Exit policy — definition version 1.3 (EXTERNAL-REVIEW-2026-09 §3.2, ADR-P6-01).
//!
//! ## The hole this closes
//!
//! Until 1.3, `ActionKind` had exactly one variant, `PlaceOrder`, and the
//! expression grammar was stateless: every expression is a comparison over the
//! *current* bar. Between them that made every path-dependent exit
//! inexpressible — stop-loss and take-profit need the entry price, a time exit
//! needs bars-since-entry, a trailing stop needs the running extreme since
//! entry. None of those is a function of the current bar.
//!
//! So every strategy the platform could author was entry-only, and the risk
//! layer's position limits were silently doing the exit logic. Position
//! management drives more P&L variance than entry timing in most systematic
//! strategies, which made this the largest single constraint on what the agent
//! could discover.
//!
//! ## Why this is not grammar
//!
//! The obvious fix is to give the grammar state. That is the wrong fix, and the
//! reason is the L0–L3 argument: expressivity belongs in the feature and model
//! layers, which already have versioning, lineage and `TrialTicket`, not in the
//! comparison operator, which has none of those. Widening the grammar widens the
//! hypothesis space in the one place the platform cannot audit.
//!
//! So exits are **typed fields, evaluated by the runtime against position
//! state**, and the grammar stays frozen. [`ExitRule`] is a closed enum: adding
//! a rule is a schema change with a version bump, not an expression somebody
//! writes at 2 a.m.
//!
//! ## Why "no exits" must be said out loud
//!
//! [`ExitPolicy`] has no `Default`, and `RiskLayerOnly` is a variant rather than
//! an absence. A strategy that delegates its exits to the risk layer is making a
//! real and sometimes correct choice; a strategy that *forgot* to have exits
//! looks identical from the outside, and did until now. Making the delegation
//! explicit puts it in the content hash, which means two revisions that differ
//! only in whether they have exits are no longer the same artifact.
//!
//! That last point is the one that matters for the ledger: `RunConfig
//! .strategy_version` hashes the definition, and an exit policy that lived
//! outside the definition would be a way for two runs with the same hash to
//! behave differently.
//!
//! ## Units — definition version 1.4 (ADR-D-09)
//!
//! Until 1.4 a distance was a fraction of the entry price and nothing else, so
//! `exit stop 1.5 atr from entry` — the most common risk rule in the domain, and
//! AGENT-011 §3's own worked example — had nowhere to go. The compiler refused it
//! by name (`unit_not_representable`) rather than converting it, because an ATR
//! distance *cannot* be converted: it is a per-bar quantity, and the fraction a
//! author computes by hand today is wrong on the next bar **and looks identical
//! in storage**.
//!
//! 1.4 adds [`DistanceUnit`] to [`ExitDistance`]. Three properties make that
//! additive rather than a rewrite of the stored form (ADR-0010):
//!
//! 1. [`DistanceUnit::FractionOfEntry`] is the `Default`, so a document that
//!    does not mention a unit means exactly what it meant at 1.0;
//! 2. the field is skipped on serialization when it is the default, so a
//!    1.0/1.2/1.3 document round-trips **byte for byte** — pinned by
//!    `a_pre_1_4_document_round_trips_byte_for_byte`;
//! 3. the wire key for the magnitude is still `fraction`, because the stored
//!    documents spell it that way. The Rust field is called `magnitude`: a
//!    reader that wants the old meaning has to ask for it through
//!    [`ExitDistance::as_fraction_of_entry`], which answers `None` for a scaled
//!    unit instead of handing back a number that would be multiplied by the
//!    entry price and be silently wrong.
//!
//! The lookback is a **typed** `period: u32`, not a string. `1.5 atr` without a
//! period is not a 14-period ATR by default; it is a refusal, because the
//! difference between `atr(14)` and `atr(96)` on an hourly bar is the difference
//! between two strategies.
//!
//! ## What the runtime owes this module
//!
//! Evaluating a scaled distance needs a number this module cannot compute: the
//! bar's ATR or sigma. So [`PositionState`] carries [`VolatilityReadings`], and
//! whoever holds the position fills them — see [`VolatilityReadings`] for the
//! exact symbol `strategy-runtime` must provide. An absent reading produces
//! [`Verdict::Unmeasured`], never [`Verdict::Holds`]: an ATR stop with no ATR is
//! an unbounded position, not a position that is fine.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// What closes a position.
///
/// Deliberately has no `Default`: see the module docs. A definition at version
/// 1.3 or later must carry one, and "the risk layer handles it" is spelled
/// [`ExitPolicy::RiskLayerOnly`] rather than left blank.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ExitPolicy {
    /// The definition declares no exit rule. Position limits in `risk_overrides`
    /// and any signal that emits flat are the entire exit logic.
    ///
    /// A legitimate choice for a strategy whose entry signal reverses cleanly.
    /// It is a variant and not an absence so that choosing it is a recorded act.
    RiskLayerOnly,
    /// Path-dependent rules evaluated against live position state each bar.
    ///
    /// The validator rejects an empty rule list: a `Rules` policy with nothing
    /// in it is `RiskLayerOnly` wearing a disguise, and the two must not hash
    /// alike.
    Rules {
        /// Evaluated in order; the first that fires closes the position. Order
        /// is therefore semantic and must never be canonicalised away by a
        /// hashing or normalisation pass.
        rules: Vec<ExitRule>,
    },
}

impl ExitPolicy {
    /// The rules, or an empty slice for [`ExitPolicy::RiskLayerOnly`].
    #[must_use]
    pub fn rules(&self) -> &[ExitRule] {
        match self {
            Self::RiskLayerOnly => &[],
            Self::Rules { rules } => rules,
        }
    }

    /// Whether this policy can ever close a position on its own.
    #[must_use]
    pub fn is_self_closing(&self) -> bool {
        !self.rules().is_empty()
    }

    /// The per-bar units this policy needs a reading for, in declaration order
    /// and without repeats. Empty for a policy whose distances are all fractions
    /// of entry, which is every policy written before 1.4.
    ///
    /// This is the list `strategy-runtime` subscribes to: see
    /// [`VolatilityReadings`].
    #[must_use]
    pub fn units(&self) -> Vec<DistanceUnit> {
        let mut out: Vec<DistanceUnit> = Vec::new();
        for rule in self.rules() {
            let unit = rule.unit();
            if !unit.is_fraction_of_entry() && !out.contains(&unit) {
                out.push(unit);
            }
        }
        out
    }
}

/// The smallest lookback the feature engine will compute a rolling statistic
/// over: `features::registry`'s `atr` and `rolling_std` both declare
/// `Param::new("n", 2, 100_000)`.
///
/// Spelled here because `domain` is below `features` in the dependency order;
/// `strategy_lang::compile`'s `the_unit_periods_match_the_engine_registry` reads
/// the registry and fails if these two ever disagree, so this is a mirror with a
/// test holding it, not a second opinion.
pub const MIN_UNIT_PERIOD: u32 = 2;
/// The largest lookback the feature engine will compute. See [`MIN_UNIT_PERIOD`].
pub const MAX_UNIT_PERIOD: u32 = 100_000;

/// What a distance is measured in (definition version 1.4, ADR-D-09).
///
/// The default is [`Self::FractionOfEntry`], which is what every distance meant
/// at 1.0 through 1.3. The two scaled units carry their own lookback because an
/// ATR without a period is not a measurement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "measure", rename_all = "snake_case")]
pub enum DistanceUnit {
    /// A positive fraction of the reference price: `0.02` is a 2% move. The
    /// only unit a document below 1.4 can express, and the meaning a document
    /// that omits the field keeps.
    #[default]
    FractionOfEntry,
    /// Multiples of the Wilder average true range over `period` bars — the
    /// engine's `atr_{period}` feature.
    Atr { period: u32 },
    /// Multiples of the sample standard deviation of close over `period` bars —
    /// the engine's `rolling_std_{period}` feature.
    Sigma { period: u32 },
}

impl DistanceUnit {
    /// A stable identifier, matching the SLv2 keyword and the serialized tag.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FractionOfEntry => "fraction_of_entry",
            Self::Atr { .. } => "atr",
            Self::Sigma { .. } => "sigma",
        }
    }

    /// Whether this is the pre-1.4 meaning. Drives `skip_serializing_if`, which
    /// is what keeps older documents byte-identical.
    #[must_use]
    pub const fn is_fraction_of_entry(&self) -> bool {
        matches!(self, Self::FractionOfEntry)
    }

    /// The lookback, for the two units that have one.
    #[must_use]
    pub const fn period(self) -> Option<u32> {
        match self {
            Self::FractionOfEntry => None,
            Self::Atr { period } | Self::Sigma { period } => Some(period),
        }
    }

    /// The feature the engine must serve for this unit to be measurable, or
    /// `None` for [`Self::FractionOfEntry`], which needs no feature.
    ///
    /// This is the string `strategy-runtime` subscribes to. It is derived here
    /// rather than spelled at the call site so that a unit and the feature that
    /// measures it cannot drift.
    #[must_use]
    pub fn feature_name(self) -> Option<String> {
        match self {
            Self::FractionOfEntry => None,
            Self::Atr { period } => Some(format!("atr_{period}")),
            Self::Sigma { period } => Some(format!("rolling_std_{period}")),
        }
    }

    /// Whether the lookback is one the feature engine will compute.
    ///
    /// A period of 0 is the failure this guards: it is not "no lookback", it is
    /// a feature name the engine refuses, discovered after the strategy is
    /// stored.
    #[must_use]
    pub const fn is_well_formed(self) -> bool {
        match self {
            Self::FractionOfEntry => true,
            Self::Atr { period } | Self::Sigma { period } => {
                period >= MIN_UNIT_PERIOD && period <= MAX_UNIT_PERIOD
            }
        }
    }
}

impl std::fmt::Display for DistanceUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FractionOfEntry => f.write_str("fraction_of_entry"),
            Self::Atr { period } => write!(f, "atr({period})"),
            Self::Sigma { period } => write!(f, "sigma({period})"),
        }
    }
}

/// How far from its reference price an exit sits, and in what unit.
///
/// Flattened into the rule that owns it, so the wire form of a fraction-of-entry
/// distance is the bare `"fraction"` key it has always been.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExitDistance {
    /// The magnitude, in [`Self::unit`].
    ///
    /// Serialized as `fraction` because that is the key in every stored
    /// document, and ADR-0010 forbids rewriting the canonical form. The Rust
    /// name is deliberately *not* `fraction`: at `unit: atr` this is a
    /// multiplier, and code that multiplies it by the entry price is wrong.
    #[serde(rename = "fraction")]
    pub magnitude: Decimal,
    /// Absent in the wire form means [`DistanceUnit::FractionOfEntry`] — the
    /// documented default, not a guess.
    #[serde(default, skip_serializing_if = "DistanceUnit::is_fraction_of_entry")]
    pub unit: DistanceUnit,
}

impl ExitDistance {
    /// A fraction of the reference price — the 1.0 meaning.
    #[must_use]
    pub const fn of_entry(fraction: Decimal) -> Self {
        Self {
            magnitude: fraction,
            unit: DistanceUnit::FractionOfEntry,
        }
    }

    /// A distance in a per-bar unit.
    #[must_use]
    pub const fn scaled(magnitude: Decimal, unit: DistanceUnit) -> Self {
        Self { magnitude, unit }
    }

    /// The magnitude **only if** it really is a fraction of the reference price.
    ///
    /// The one accessor a pre-1.4 reader should use. It answers `None` for a
    /// scaled unit rather than handing back a number that looks like a fraction,
    /// which is the whole reason the Rust field is not called `fraction`.
    #[must_use]
    pub const fn as_fraction_of_entry(&self) -> Option<Decimal> {
        match self.unit {
            DistanceUnit::FractionOfEntry => Some(self.magnitude),
            _ => None,
        }
    }

    /// Whether the magnitude and its unit are in range.
    ///
    /// The bounds differ **by unit**, and that is the point: `1.5` as a fraction
    /// of entry is a stop that can never fire on a long, while `1.5 atr` is an
    /// ordinary stop. Holding the old `(0, 1]` bound over both units would have
    /// rejected the most common risk rule in the domain.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        if !self.unit.is_well_formed() {
            return false;
        }
        match self.unit {
            DistanceUnit::FractionOfEntry => {
                self.magnitude > Decimal::ZERO && self.magnitude <= Decimal::ONE
            }
            DistanceUnit::Atr { .. } | DistanceUnit::Sigma { .. } => self.magnitude > Decimal::ZERO,
        }
    }
}

/// One path-dependent exit condition.
///
/// Every variant is a function of `(entry, extreme-since-entry, bars-held,
/// current)` — the four pieces of position state the grammar cannot see, plus,
/// from 1.4, the bar's volatility for a scaled distance. A rule that needs a
/// sixth is a schema change, on purpose.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum ExitRule {
    /// Close when the position is down `distance` from the entry price.
    /// `0.02` fraction-of-entry = a 2% adverse move; `1.5 atr(14)` = one and a
    /// half average true ranges. Never an absolute price, so the rule survives a
    /// parameter sweep across instruments at different price scales.
    StopLoss {
        #[serde(flatten)]
        distance: ExitDistance,
    },
    /// Close when the position is up `distance` from the entry price.
    TakeProfit {
        #[serde(flatten)]
        distance: ExitDistance,
    },
    /// Close when price retraces `distance` from the best level reached since
    /// entry. Ratchets one way only — the reference is the running extreme, not
    /// the entry.
    TrailingStop {
        #[serde(flatten)]
        distance: ExitDistance,
    },
    /// Close after `bars` bars in the position, whatever the price has done.
    /// The exit that makes a holding period a declared quantity rather than an
    /// emergent one, which is what the tradeable-frontier check needs to read.
    MaxHoldingBars { bars: u32 },
    /// Close when the named signal fires. The one rule expressible in the old
    /// world; kept here so that a definition's exits are all in one place
    /// rather than split between `actions` and this block.
    OnSignal { emit: String },
}

impl ExitRule {
    /// A stable identifier for logging and for the reason string on a fill.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::StopLoss { .. } => "stop_loss",
            Self::TakeProfit { .. } => "take_profit",
            Self::TrailingStop { .. } => "trailing_stop",
            Self::MaxHoldingBars { .. } => "max_holding_bars",
            Self::OnSignal { .. } => "on_signal",
        }
    }

    /// The declared holding-period bound this rule implies, if it implies one.
    ///
    /// Only [`ExitRule::MaxHoldingBars`] does. A stop-loss bounds *loss*, not
    /// time, and reading it as a holding period is how a strategy ends up being
    /// admitted below the tradeable frontier.
    #[must_use]
    pub const fn holding_bound(&self) -> Option<u32> {
        match self {
            Self::MaxHoldingBars { bars } => Some(*bars),
            _ => None,
        }
    }

    /// Whether the rule's parameters are in range. A fraction outside `(0, 1]`
    /// is not a conservative stop, it is a typo: a stop at 1.5 *of entry* can
    /// never fire on a long, and one at 0 fires on the entry bar. See
    /// [`ExitDistance::is_well_formed`] for why the bound depends on the unit.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        match self {
            Self::StopLoss { distance }
            | Self::TakeProfit { distance }
            | Self::TrailingStop { distance } => distance.is_well_formed(),
            Self::MaxHoldingBars { bars } => *bars > 0,
            Self::OnSignal { emit } => !emit.trim().is_empty(),
        }
    }

    /// The distance this rule measures, for the three rules that measure one.
    #[must_use]
    pub const fn distance(&self) -> Option<&ExitDistance> {
        match self {
            Self::StopLoss { distance }
            | Self::TakeProfit { distance }
            | Self::TrailingStop { distance } => Some(distance),
            Self::MaxHoldingBars { .. } | Self::OnSignal { .. } => None,
        }
    }

    /// The unit this rule's distance is in, or [`DistanceUnit::FractionOfEntry`]
    /// for the rules that have no distance.
    #[must_use]
    pub const fn unit(&self) -> DistanceUnit {
        match self.distance() {
            Some(d) => d.unit,
            None => DistanceUnit::FractionOfEntry,
        }
    }
}

/// A per-bar volatility reading, keyed by the unit it measures.
///
/// Constructed through [`Self::new`], which refuses a non-positive value. That
/// refusal is the flat-bar case: an instrument whose bars do not move has an ATR
/// of zero, and a zero-width stop fires on the entry bar of every position. A
/// zero is not a small measurement, it is the absence of one, and it belongs in
/// [`Verdict::Unmeasured`] rather than in a fill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolatilityReading {
    unit: DistanceUnit,
    value: Decimal,
}

impl VolatilityReading {
    /// A reading for `unit`, or `None` when `value` is not a usable measurement.
    #[must_use]
    pub fn new(unit: DistanceUnit, value: Decimal) -> Option<Self> {
        (value > Decimal::ZERO && !unit.is_fraction_of_entry()).then_some(Self { unit, value })
    }

    #[must_use]
    pub const fn unit(&self) -> DistanceUnit {
        self.unit
    }

    #[must_use]
    pub const fn value(&self) -> Decimal {
        self.value
    }
}

/// The per-bar volatility an exit with a scaled distance needs.
///
/// # The symbol `strategy-runtime` must provide
///
/// This type is the whole interface between the schema and the runtime.
/// `strategy-runtime` must add, on whatever holds the open position:
///
/// ```text
/// fn volatility_readings(&self, units: &[DistanceUnit]) -> VolatilityReadings
/// ```
///
/// filling it from the feature named by [`DistanceUnit::feature_name`] —
/// `atr_{period}` or `rolling_std_{period}` — for the bar being evaluated, and
/// passing it in [`PositionState::volatility`]. The units a policy needs are
/// [`ExitPolicy::units`]; the features to subscribe to are their
/// [`DistanceUnit::feature_name`]s.
///
/// A missing reading is not an error the runtime may swallow: [`first_triggered`]
/// returns [`Triggered::Unmeasured`] and the runtime has to decide out loud what
/// an unguarded position does.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VolatilityReadings(Vec<VolatilityReading>);

impl VolatilityReadings {
    /// No readings. Correct for a policy with no scaled distance, and the reason
    /// a pre-1.4 caller needs to say nothing new.
    #[must_use]
    pub const fn none() -> Self {
        Self(Vec::new())
    }

    /// Record a reading. A later reading for the same unit replaces an earlier
    /// one; a value that is not a measurement is refused, and the return says
    /// which happened rather than leaving the caller to assume.
    pub fn insert(&mut self, unit: DistanceUnit, value: Decimal) -> bool {
        match VolatilityReading::new(unit, value) {
            Some(r) => {
                self.0.retain(|x| x.unit != unit);
                self.0.push(r);
                true
            }
            None => false,
        }
    }

    /// The reading for `unit`, if one was supplied.
    #[must_use]
    pub fn get(&self, unit: DistanceUnit) -> Option<Decimal> {
        self.0
            .iter()
            .find(|r| r.unit == unit)
            .map(VolatilityReading::value)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What one rule says about one bar.
///
/// Three-valued on purpose. Collapsing [`Self::Unmeasured`] into [`Self::Holds`]
/// would make an ATR stop with no ATR indistinguishable from an ATR stop that is
/// comfortably far away, and the position would stay open forever.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The rule closes the position on this bar.
    Fires,
    /// The rule does not close the position on this bar.
    Holds,
    /// The rule's distance is in this unit and no reading for it was supplied.
    Unmeasured(DistanceUnit),
}

/// What a whole policy says about one bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Triggered<'a> {
    /// This rule fired, and it is the first one that did.
    Fired(&'a ExitRule),
    /// No rule fired, and every rule could be evaluated.
    NothingFired,
    /// Evaluation stopped at this rule because its unit had no reading.
    ///
    /// It stops rather than skipping, because declaration order is semantic: a
    /// later rule that fires would be reported as *the* exit, when the rule
    /// ahead of it might have fired first.
    Unmeasured {
        rule: &'a ExitRule,
        unit: DistanceUnit,
    },
}

/// Which way a position faces. Exits are asymmetric and the sign is not
/// recoverable from the prices alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionSide {
    Long,
    Short,
}

/// The position state an exit rule reads. Assembled by whoever holds the
/// position — the live runtime or the simulator — so that both evaluate exits
/// through the same function and cannot drift apart.
#[derive(Clone, Debug, PartialEq)]
pub struct PositionState {
    pub side: PositionSide,
    /// Price the position was opened at.
    pub entry_price: Decimal,
    /// Most favourable price reached since entry: the high for a long, the low
    /// for a short. The trailing stop's reference.
    pub favourable_extreme: Decimal,
    /// Bars elapsed since the entry bar. The entry bar itself is 0.
    pub bars_held: u32,
    /// Price being evaluated against.
    pub current_price: Decimal,
    /// Signals emitted on this bar, for [`ExitRule::OnSignal`].
    pub signals: Vec<String>,
    /// The bar's volatility, for a rule whose distance is in a per-bar unit
    /// (1.4). [`VolatilityReadings::none()`] is correct for a policy with no
    /// such rule — and is what [`ExitPolicy::units`] returning empty means.
    pub volatility: VolatilityReadings,
}

impl PositionState {
    /// Signed return from entry, as a fraction: positive is profit for the side
    /// the position is on.
    #[must_use]
    pub fn pnl_fraction(&self) -> Option<Decimal> {
        (self.entry_price > Decimal::ZERO).then(|| {
            let raw = (self.current_price - self.entry_price) / self.entry_price;
            match self.side {
                PositionSide::Long => raw,
                PositionSide::Short => -raw,
            }
        })
    }

    /// Retracement from the favourable extreme, as a positive fraction.
    #[must_use]
    pub fn retracement_fraction(&self) -> Option<Decimal> {
        (self.favourable_extreme > Decimal::ZERO).then(|| {
            let raw = (self.current_price - self.favourable_extreme) / self.favourable_extreme;
            match self.side {
                PositionSide::Long => -raw,
                PositionSide::Short => raw,
            }
        })
    }
}

/// Evaluate a policy against position state, returning the first rule that
/// fires.
///
/// Order is the declared order. A policy whose stop is listed after its target
/// behaves differently from one whose stop is first on a bar where both are
/// breached, and that is the author's decision to make — so nothing here sorts.
#[must_use]
pub fn first_triggered<'a>(policy: &'a ExitPolicy, state: &PositionState) -> Triggered<'a> {
    for rule in policy.rules() {
        match triggers(rule, state) {
            Verdict::Fires => return Triggered::Fired(rule),
            Verdict::Holds => {}
            Verdict::Unmeasured(unit) => return Triggered::Unmeasured { rule, unit },
        }
    }
    Triggered::NothingFired
}

/// What one rule says about this state.
///
/// Returns [`Verdict::Unmeasured`] rather than a boolean when the rule's distance
/// is in a per-bar unit and `state.volatility` has no reading for it. There is no
/// fallback: the reason 1.4 exists is that a per-bar distance has no fraction of
/// entry that means the same thing.
#[must_use]
pub fn triggers(rule: &ExitRule, state: &PositionState) -> Verdict {
    match rule {
        ExitRule::StopLoss { distance } => {
            match resolve(distance, state.entry_price, &state.volatility) {
                Err(unit) => Verdict::Unmeasured(unit),
                Ok(f) => fires(state.pnl_fraction().is_some_and(|p| p <= -f)),
            }
        }
        ExitRule::TakeProfit { distance } => {
            match resolve(distance, state.entry_price, &state.volatility) {
                Err(unit) => Verdict::Unmeasured(unit),
                Ok(f) => fires(state.pnl_fraction().is_some_and(|p| p >= f)),
            }
        }
        ExitRule::TrailingStop { distance } => {
            // The reference is the extreme, not the entry: `retracement_fraction`
            // divides by `favourable_extreme`, so a scaled distance has to be
            // turned into a fraction of the same denominator or the band moves.
            match resolve(distance, state.favourable_extreme, &state.volatility) {
                Err(unit) => Verdict::Unmeasured(unit),
                Ok(f) => fires(state.retracement_fraction().is_some_and(|r| r >= f)),
            }
        }
        ExitRule::MaxHoldingBars { bars } => fires(state.bars_held >= *bars),
        ExitRule::OnSignal { emit } => fires(state.signals.iter().any(|s| s == emit)),
    }
}

const fn fires(b: bool) -> Verdict {
    if b {
        Verdict::Fires
    } else {
        Verdict::Holds
    }
}

/// A distance as a fraction of `reference`, or the unit that had no reading.
///
/// `magnitude * atr / reference` is the conversion, done here and only here so
/// that the live runtime and the simulator cannot disagree about it. A
/// non-positive `reference` yields `Ok(magnitude)`-free behaviour: the caller's
/// `pnl_fraction` / `retracement_fraction` already answer `None` for that, and
/// inventing a fraction from a zero price would be a division by zero dressed up.
impl ExitDistance {
    /// This distance as a fraction of `reference`, or the unit that had no
    /// reading.
    ///
    /// The public form of [`resolve`], added for `crates/strategy-runtime`,
    /// which has to price a fill level and cannot reach a private function.
    /// Returning `Result<_, DistanceUnit>` rather than an `Option` is the point:
    /// a caller that cannot resolve a distance learns *which* unit it was
    /// missing, and `Err` is impossible to spend as a number the way a `None`
    /// folded through `unwrap_or` would be.
    ///
    /// # Errors
    ///
    /// The distance is in a per-bar unit and `vol` holds no reading for it, or
    /// `reference` is not positive.
    pub fn as_fraction(
        &self,
        reference: Decimal,
        vol: &VolatilityReadings,
    ) -> Result<Decimal, DistanceUnit> {
        resolve(self, reference, vol)
    }
}

fn resolve(
    distance: &ExitDistance,
    reference: Decimal,
    vol: &VolatilityReadings,
) -> Result<Decimal, DistanceUnit> {
    match distance.unit {
        DistanceUnit::FractionOfEntry => Ok(distance.magnitude),
        unit => {
            let Some(measure) = vol.get(unit) else {
                return Err(unit);
            };
            if reference <= Decimal::ZERO {
                // Nothing can be a fraction of a non-positive reference price.
                // Reported as unmeasured rather than as a fraction, because the
                // alternative is a number that would fire or not by accident.
                return Err(unit);
            }
            Ok(distance.magnitude * measure / reference)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn long_at(entry: f64, now: f64) -> PositionState {
        PositionState {
            side: PositionSide::Long,
            entry_price: Decimal::try_from(entry).unwrap(),
            favourable_extreme: Decimal::try_from(entry.max(now)).unwrap(),
            bars_held: 1,
            current_price: Decimal::try_from(now).unwrap(),
            signals: vec![],
            volatility: VolatilityReadings::none(),
        }
    }

    fn short_at(entry: f64, now: f64) -> PositionState {
        PositionState {
            side: PositionSide::Short,
            entry_price: Decimal::try_from(entry).unwrap(),
            favourable_extreme: Decimal::try_from(entry.min(now)).unwrap(),
            bars_held: 1,
            current_price: Decimal::try_from(now).unwrap(),
            signals: vec![],
            volatility: VolatilityReadings::none(),
        }
    }

    fn stop(fraction: Decimal) -> ExitRule {
        ExitRule::StopLoss {
            distance: ExitDistance::of_entry(fraction),
        }
    }

    fn target(fraction: Decimal) -> ExitRule {
        ExitRule::TakeProfit {
            distance: ExitDistance::of_entry(fraction),
        }
    }

    fn trail(fraction: Decimal) -> ExitRule {
        ExitRule::TrailingStop {
            distance: ExitDistance::of_entry(fraction),
        }
    }

    fn atr_stop(multiple: Decimal, period: u32) -> ExitRule {
        ExitRule::StopLoss {
            distance: ExitDistance::scaled(multiple, DistanceUnit::Atr { period }),
        }
    }

    fn fired(rule: &ExitRule, state: &PositionState) -> bool {
        match triggers(rule, state) {
            Verdict::Fires => true,
            Verdict::Holds => false,
            Verdict::Unmeasured(u) => panic!("unmeasured {u}"),
        }
    }

    // ── the policy itself ────────────────────────────────────────────────────

    /// The point of the whole module: "no exits" is a statement, not a blank.
    #[test]
    fn risk_layer_only_is_a_variant_not_an_absence() {
        let json = serde_json::to_value(&ExitPolicy::RiskLayerOnly).unwrap();
        assert_eq!(json, serde_json::json!({ "mode": "risk_layer_only" }));
    }

    /// Two policies that behave differently must not serialise alike, because
    /// the definition hash is what makes a `strategy_version` mean anything.
    #[test]
    fn delegating_and_declaring_do_not_hash_alike() {
        let delegated = serde_json::to_string(&ExitPolicy::RiskLayerOnly).unwrap();
        let declared = serde_json::to_string(&ExitPolicy::Rules {
            rules: vec![stop(dec!(0.02))],
        })
        .unwrap();
        assert_ne!(delegated, declared);
    }

    #[test]
    fn a_policy_with_no_rules_cannot_close_a_position() {
        assert!(!ExitPolicy::RiskLayerOnly.is_self_closing());
        assert!(!ExitPolicy::Rules { rules: vec![] }.is_self_closing());
        assert!(ExitPolicy::Rules {
            rules: vec![ExitRule::MaxHoldingBars { bars: 10 }]
        }
        .is_self_closing());
    }

    // ── stops and targets ────────────────────────────────────────────────────

    #[test]
    fn a_long_stop_fires_on_an_adverse_move_and_not_a_favourable_one() {
        let s = stop(dec!(0.02));
        assert!(fired(&s, &long_at(100.0, 98.0)));
        assert!(fired(&s, &long_at(100.0, 97.0)));
        assert!(!fired(&s, &long_at(100.0, 98.5)));
        assert!(!fired(&s, &long_at(100.0, 102.0)));
    }

    /// The sign error this catches is the classic one: a short's stop is above
    /// its entry, and a rule written from the long's point of view never fires.
    #[test]
    fn a_short_stop_fires_when_price_rises() {
        let s = stop(dec!(0.02));
        assert!(fired(&s, &short_at(100.0, 102.0)));
        assert!(!fired(&s, &short_at(100.0, 98.0)));
    }

    #[test]
    fn a_short_target_fires_when_price_falls() {
        let tp = target(dec!(0.02));
        assert!(fired(&tp, &short_at(100.0, 98.0)));
        assert!(!fired(&tp, &short_at(100.0, 102.0)));
    }

    // ── the trailing stop ────────────────────────────────────────────────────

    /// A trailing stop measures from the extreme, not the entry. A position
    /// that ran to +10% and gave back 3% must exit even though it is still
    /// profitable — the case a stop-loss cannot express and the reason the rule
    /// exists.
    #[test]
    fn a_trailing_stop_measures_from_the_extreme_not_the_entry() {
        let t = trail(dec!(0.03));
        let mut s = long_at(100.0, 107.0);
        s.favourable_extreme = dec!(110);
        assert!(!fired(&t, &s), "110 to 107 is 2.73%, under the 3% band");

        s.current_price = dec!(106);
        assert!(
            fired(&t, &s),
            "110 to 106 is a 3.6% retrace and must fire even though the position is still \
             +6% on the entry — the case a stop-loss cannot express"
        );

        s.current_price = dec!(109);
        assert!(!fired(&t, &s), "110 to 109 is under 1%");
    }

    #[test]
    fn a_short_trailing_stop_ratchets_downward() {
        let t = trail(dec!(0.03));
        let mut s = short_at(100.0, 93.0);
        s.favourable_extreme = dec!(90);
        assert!(fired(&t, &s), "90 to 93 is a 3.3% retrace on a short");
        s.current_price = dec!(91);
        assert!(!fired(&t, &s));
    }

    // ── time and signal ──────────────────────────────────────────────────────

    #[test]
    fn the_time_exit_fires_at_the_bound_not_after_it() {
        let t = ExitRule::MaxHoldingBars { bars: 5 };
        let mut s = long_at(100.0, 100.0);
        s.bars_held = 4;
        assert!(!fired(&t, &s));
        s.bars_held = 5;
        assert!(fired(&t, &s));
    }

    #[test]
    fn the_signal_exit_matches_by_name() {
        let r = ExitRule::OnSignal {
            emit: "flat".into(),
        };
        let mut s = long_at(100.0, 100.0);
        assert!(!fired(&r, &s));
        s.signals = vec!["long".into()];
        assert!(!fired(&r, &s));
        s.signals = vec!["long".into(), "flat".into()];
        assert!(fired(&r, &s));
    }

    // ── ordering ─────────────────────────────────────────────────────────────

    /// Declared order is semantic. On a bar that gaps through both the stop and
    /// the target, which one you are deemed to have hit is the difference
    /// between a win and a loss, and nothing in the platform is allowed to
    /// reorder that silently.
    #[test]
    fn declaration_order_decides_which_rule_fires_on_a_bar_that_breaches_both() {
        let mut s = long_at(100.0, 95.0);
        s.favourable_extreme = dec!(106);

        let stop_first = ExitPolicy::Rules {
            rules: vec![stop(dec!(0.02)), trail(dec!(0.03))],
        };
        let trail_first = ExitPolicy::Rules {
            rules: vec![trail(dec!(0.03)), stop(dec!(0.02))],
        };
        assert_eq!(
            first_triggered(&stop_first, &s),
            Triggered::Fired(&stop(dec!(0.02)))
        );
        assert_eq!(
            first_triggered(&trail_first, &s),
            Triggered::Fired(&trail(dec!(0.03)))
        );
    }

    #[test]
    fn nothing_fires_when_nothing_is_breached() {
        let p = ExitPolicy::Rules {
            rules: vec![stop(dec!(0.05)), ExitRule::MaxHoldingBars { bars: 100 }],
        };
        assert_eq!(
            first_triggered(&p, &long_at(100.0, 101.0)),
            Triggered::NothingFired
        );
    }

    // ── well-formedness ──────────────────────────────────────────────────────

    /// A stop at 150% of entry can never fire on a long and a stop at 0% fires
    /// on the entry bar. Both are typos, and the validator refuses both.
    #[test]
    fn fractions_outside_zero_to_one_are_not_well_formed() {
        assert!(stop(dec!(0.02)).is_well_formed());
        assert!(stop(dec!(1)).is_well_formed());
        assert!(!stop(dec!(1.5)).is_well_formed());
        assert!(!stop(Decimal::ZERO).is_well_formed());
        assert!(!target(dec!(-0.1)).is_well_formed());
        assert!(!ExitRule::MaxHoldingBars { bars: 0 }.is_well_formed());
        assert!(!ExitRule::OnSignal { emit: "  ".into() }.is_well_formed());
    }

    /// Only the time exit bounds holding period. Reading a stop-loss as one is
    /// how a candidate gets admitted below the tradeable frontier.
    #[test]
    fn only_the_time_exit_bounds_the_holding_period() {
        assert_eq!(
            ExitRule::MaxHoldingBars { bars: 30 }.holding_bound(),
            Some(30)
        );
        assert_eq!(stop(dec!(0.02)).holding_bound(), None);
        assert_eq!(trail(dec!(0.02)).holding_bound(), None);
    }

    #[test]
    fn rules_round_trip_through_json_with_their_tag() {
        let p = ExitPolicy::Rules {
            rules: vec![
                stop(dec!(0.02)),
                trail(dec!(0.03)),
                ExitRule::MaxHoldingBars { bars: 240 },
                ExitRule::OnSignal {
                    emit: "flat".into(),
                },
            ],
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains(r#""rule":"stop_loss""#), "{s}");
        assert_eq!(serde_json::from_str::<ExitPolicy>(&s).unwrap(), p);
    }

    #[test]
    fn a_zero_entry_price_yields_no_verdict_rather_than_a_division_by_zero() {
        let mut s = long_at(100.0, 100.0);
        s.entry_price = Decimal::ZERO;
        assert!(s.pnl_fraction().is_none());
        assert!(!fired(&stop(dec!(0.02)), &s));
    }

    // ── units (1.4, ADR-D-09) ────────────────────────────────────────────────

    /// **The compatibility proof.** These are the shapes the stored
    /// `strategy_definitions.definition_json` can hold at 1.0 through 1.3.
    /// Adding a field to `ExitRule` is only additive if the text comes back out
    /// identical, byte for byte.
    #[test]
    fn a_pre_1_4_document_round_trips_byte_for_byte() {
        for stored in [
            r#"{"mode":"risk_layer_only"}"#,
            r#"{"mode":"rules","rules":[{"rule":"stop_loss","fraction":"0.02"}]}"#,
            r#"{"mode":"rules","rules":[{"rule":"stop_loss","fraction":"0.02"},{"rule":"take_profit","fraction":"0.06"},{"rule":"trailing_stop","fraction":"0.03"},{"rule":"max_holding_bars","bars":240},{"rule":"on_signal","emit":"flat"}]}"#,
        ] {
            let policy: ExitPolicy = serde_json::from_str(stored)
                .unwrap_or_else(|e| panic!("{stored} must still parse: {e}"));
            assert_eq!(
                serde_json::to_string(&policy).unwrap(),
                stored,
                "a stored document must come back out unchanged"
            );
        }
    }

    /// An absent unit is the documented default, and the default is what every
    /// document below 1.4 meant.
    #[test]
    fn an_absent_unit_is_fraction_of_entry_and_is_not_written_back() {
        let r: ExitRule =
            serde_json::from_str(r#"{"rule":"stop_loss","fraction":"0.02"}"#).unwrap();
        assert_eq!(r.unit(), DistanceUnit::FractionOfEntry);
        assert_eq!(
            r.distance().unwrap().as_fraction_of_entry(),
            Some(dec!(0.02))
        );
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"rule":"stop_loss","fraction":"0.02"}"#
        );
    }

    /// The unit and its period are in the wire form, and the period is a number
    /// there — not a string somebody parses later.
    #[test]
    fn a_scaled_distance_round_trips_with_its_period() {
        let r = atr_stop(dec!(1.5), 14);
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(
            s,
            r#"{"rule":"stop_loss","fraction":"1.5","unit":{"measure":"atr","period":14}}"#
        );
        assert_eq!(serde_json::from_str::<ExitRule>(&s).unwrap(), r);
        assert_eq!(r.distance().unwrap().as_fraction_of_entry(), None);
    }

    /// Two rules that behave differently must not serialise alike: a 150% stop
    /// and a 1.5-ATR stop are different artifacts and the hash has to see that.
    #[test]
    fn the_unit_is_part_of_the_stored_form() {
        let a = serde_json::to_string(&stop(dec!(1.5))).unwrap();
        let b = serde_json::to_string(&atr_stop(dec!(1.5), 14)).unwrap();
        assert_ne!(a, b);
    }

    /// The reason the `(0, 1]` bound could not simply be kept: `1.5` is a typo as
    /// a fraction of entry and the single most ordinary stop in the domain as an
    /// ATR multiple.
    #[test]
    fn one_and_a_half_is_a_typo_as_a_fraction_and_ordinary_as_an_atr_multiple() {
        assert!(!stop(dec!(1.5)).is_well_formed());
        assert!(atr_stop(dec!(1.5), 14).is_well_formed());
        assert!(!atr_stop(Decimal::ZERO, 14).is_well_formed());
        assert!(!atr_stop(dec!(-1.5), 14).is_well_formed());
    }

    /// A period is a typed lookback the engine will actually compute. Zero is
    /// not "no lookback": it is `atr_0`, a feature name nothing serves.
    #[test]
    fn a_period_outside_the_engine_range_is_not_well_formed() {
        for bad in [0, 1, MAX_UNIT_PERIOD + 1] {
            assert!(
                !DistanceUnit::Atr { period: bad }.is_well_formed(),
                "atr({bad}) must be refused"
            );
            assert!(!DistanceUnit::Sigma { period: bad }.is_well_formed());
            assert!(!atr_stop(dec!(1.5), bad).is_well_formed());
        }
        assert!(DistanceUnit::Atr {
            period: MIN_UNIT_PERIOD
        }
        .is_well_formed());
        assert!(DistanceUnit::Atr {
            period: MAX_UNIT_PERIOD
        }
        .is_well_formed());
    }

    /// The units a policy needs, and the feature each one names. This is the
    /// list the runtime subscribes to.
    #[test]
    fn the_units_a_policy_needs_name_the_engine_features() {
        let p = ExitPolicy::Rules {
            rules: vec![
                atr_stop(dec!(1.5), 14),
                ExitRule::TakeProfit {
                    distance: ExitDistance::scaled(dec!(3), DistanceUnit::Atr { period: 14 }),
                },
                ExitRule::TrailingStop {
                    distance: ExitDistance::scaled(dec!(2), DistanceUnit::Sigma { period: 20 }),
                },
                stop(dec!(0.02)),
            ],
        };
        assert_eq!(
            p.units(),
            vec![
                DistanceUnit::Atr { period: 14 },
                DistanceUnit::Sigma { period: 20 }
            ],
            "the same unit twice is one subscription, and a fraction needs none"
        );
        assert_eq!(
            p.units()
                .iter()
                .filter_map(|u| u.feature_name())
                .collect::<Vec<_>>(),
            vec!["atr_14".to_string(), "rolling_std_20".to_string()]
        );
        assert!(ExitPolicy::Rules {
            rules: vec![stop(dec!(0.02))]
        }
        .units()
        .is_empty());
        assert!(ExitPolicy::RiskLayerOnly.units().is_empty());
    }

    /// `1.5 atr` of a 2.00 ATR on a 100 entry is a stop at 97, and the fraction
    /// it becomes is `1.5 * 2 / 100`. Computed here and only here so the live
    /// runtime and the simulator cannot disagree.
    #[test]
    fn an_atr_stop_fires_at_entry_minus_multiple_times_atr() {
        let rule = atr_stop(dec!(1.5), 14);
        let with_atr = |now: f64| {
            let mut s = long_at(100.0, now);
            assert!(s
                .volatility
                .insert(DistanceUnit::Atr { period: 14 }, dec!(2)));
            s
        };
        assert!(!fired(&rule, &with_atr(97.5)));
        assert!(fired(&rule, &with_atr(97.0)));
        assert!(fired(&rule, &with_atr(96.0)));
    }

    /// The rule the whole three-valued verdict exists for. An ATR stop whose ATR
    /// was not supplied is **not** a stop that is comfortably far away.
    #[test]
    fn an_atr_stop_with_no_atr_is_unmeasured_and_never_held() {
        let rule = atr_stop(dec!(1.5), 14);
        // Far from any plausible stop, and still not `Holds`.
        assert_eq!(
            triggers(&rule, &long_at(100.0, 100.0)),
            Verdict::Unmeasured(DistanceUnit::Atr { period: 14 })
        );
        // A reading for a *different* period is not a reading for this one.
        let mut s = long_at(100.0, 90.0);
        assert!(s
            .volatility
            .insert(DistanceUnit::Atr { period: 20 }, dec!(2)));
        assert_eq!(
            triggers(&rule, &s),
            Verdict::Unmeasured(DistanceUnit::Atr { period: 14 })
        );
    }

    /// The flat-bars case this repository already has a name for: an instrument
    /// whose bars do not move has an ATR of zero, and a zero-width stop closes
    /// every position on its entry bar. Zero is the absence of a measurement.
    #[test]
    fn a_flat_bar_atr_of_zero_is_not_a_measurement() {
        let mut v = VolatilityReadings::none();
        assert!(!v.insert(DistanceUnit::Atr { period: 14 }, Decimal::ZERO));
        assert!(!v.insert(DistanceUnit::Atr { period: 14 }, dec!(-1)));
        assert!(v.is_empty());
        assert!(VolatilityReading::new(DistanceUnit::Atr { period: 14 }, Decimal::ZERO).is_none());
        // And `fraction_of_entry` is not a thing that gets measured at all.
        assert!(!v.insert(DistanceUnit::FractionOfEntry, dec!(2)));
        // A real reading replaces an earlier one for the same unit.
        assert!(v.insert(DistanceUnit::Atr { period: 14 }, dec!(2)));
        assert!(v.insert(DistanceUnit::Atr { period: 14 }, dec!(3)));
        assert_eq!(v.get(DistanceUnit::Atr { period: 14 }), Some(dec!(3)));
    }

    /// Declaration order is semantic, so an unmeasurable rule stops the scan
    /// instead of being skipped: the rule behind it might not have been the
    /// first to fire.
    #[test]
    fn evaluation_stops_at_the_first_unmeasured_rule() {
        let scaled = atr_stop(dec!(1.5), 14);
        let time = ExitRule::MaxHoldingBars { bars: 1 };
        let p = ExitPolicy::Rules {
            rules: vec![scaled.clone(), time.clone()],
        };
        let s = long_at(100.0, 100.0);
        assert_eq!(
            first_triggered(&p, &s),
            Triggered::Unmeasured {
                rule: &scaled,
                unit: DistanceUnit::Atr { period: 14 }
            },
            "the time exit is satisfied, and reporting it would name the wrong exit"
        );
        // Reversed, the measurable rule ahead of it fires and nothing is unknown.
        let q = ExitPolicy::Rules {
            rules: vec![time.clone(), scaled],
        };
        assert_eq!(first_triggered(&q, &s), Triggered::Fired(&time));
    }

    /// A scaled trailing stop is a fraction of the *extreme*, not of the entry:
    /// dividing by the wrong denominator moves the band by the open profit.
    #[test]
    fn a_scaled_trailing_stop_measures_from_the_extreme() {
        let rule = ExitRule::TrailingStop {
            distance: ExitDistance::scaled(dec!(2), DistanceUnit::Atr { period: 14 }),
        };
        let mut s = long_at(100.0, 106.0);
        s.favourable_extreme = dec!(110);
        assert!(s
            .volatility
            .insert(DistanceUnit::Atr { period: 14 }, dec!(2)));
        // 2 x 2.00 = 4.00 off the 110 extreme is 106: the boundary fires.
        assert!(fired(&rule, &s));
        s.current_price = dec!(106.5);
        assert!(!fired(&rule, &s));
    }

    /// A non-positive reference price has no fraction, and a scaled distance
    /// must not be reported as held because of it.
    #[test]
    fn a_zero_reference_price_is_unmeasured_rather_than_held() {
        let rule = atr_stop(dec!(1.5), 14);
        let mut s = long_at(100.0, 90.0);
        s.entry_price = Decimal::ZERO;
        assert!(s
            .volatility
            .insert(DistanceUnit::Atr { period: 14 }, dec!(2)));
        assert_eq!(
            triggers(&rule, &s),
            Verdict::Unmeasured(DistanceUnit::Atr { period: 14 })
        );
    }

    #[test]
    fn a_unit_displays_with_its_period() {
        assert_eq!(DistanceUnit::Atr { period: 14 }.to_string(), "atr(14)");
        assert_eq!(DistanceUnit::Sigma { period: 20 }.to_string(), "sigma(20)");
        assert_eq!(
            DistanceUnit::FractionOfEntry.to_string(),
            "fraction_of_entry"
        );
        assert_eq!(DistanceUnit::Atr { period: 14 }.as_str(), "atr");
        assert_eq!(DistanceUnit::Atr { period: 14 }.period(), Some(14));
        assert_eq!(DistanceUnit::FractionOfEntry.period(), None);
        assert_eq!(DistanceUnit::FractionOfEntry.feature_name(), None);
    }
}
