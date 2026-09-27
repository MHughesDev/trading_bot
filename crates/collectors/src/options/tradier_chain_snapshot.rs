//! Equity option chain snapshots on the Tradier collector (DATA-005 §11 S8;
//! DA-12; Set P, P-0.5).
//!
//! # What this is, and what it deliberately is not
//!
//! This collects **snapshots**, not an equities onboarding. Every question the
//! equities schema raises — corporate actions, INV-03 adjustment factors,
//! surrogate instrument ids — stays closed behind roadmap gate H-6, and a static
//! test in this file fails the build if this module ever starts answering one of
//! them (see [`tests::no_corporate_action_handling_is_introduced_by_this_task`]).
//!
//! The storage half of this task shipped long before the collector did:
//! `option_chain_snapshots` (migration 0098 / `clickhouse/08_forward_series.sql`),
//! `storage::clickhouse::forward::write_chain_snapshot`, the
//! `equity.option_chains` budget in `config/data_retention.toml`, the
//! `tradier_chain_rest` source row, [`VenueLimit::tradier_market_data`] and
//! [`SourceDecl::tradier_chain`] were all in the tree with **no caller**. This
//! module is that caller.
//!
//! It is built and tested and **not switched on**: `equity.option_chains` carries
//! `enabled = false`, and Tradier needs a credential a human has to supply.
//!
//! # The timestamps, which are the whole point
//!
//! Tradier's data is delayed. That produces the one distinction this task exists
//! to get right: **the row is observed, but observed late, and those are not the
//! same fact.**
//!
//! * `event_time` is the market instant the quotes describe — the cadence slot,
//!   [`snapshot_slot`].
//! * `knowledge_time` is the platform's **receipt**, and it is
//!   [`Knowledge::Observed`]: we really did hold these bytes at a known instant,
//!   and there is forward evidence in saying so (Set P D-P3).
//! * The declared 900 s lag is what separates them. A punctual poll for the 14:00
//!   slot lands at 14:15, so `knowledge_time == event_time + 900 s`, which is
//!   P-0.5's acceptance clause stated as arithmetic. A late poll lands later
//!   still, which is the honest direction: we knew *later*, never sooner.
//!
//! The alternative — `event_time = the receipt` — would be lookahead by a quarter
//! of an hour. It would assert that the market state we were handed at 14:15 was
//! the market state *at* 14:15, when Tradier had in fact shown us 14:00. A
//! strategy reading that row would act on fifteen minutes it could not have had.
//! [`snapshot_at`] refuses any row whose receipt precedes `event_time + the
//! declared lag`, because a delayed vendor physically cannot have published sooner
//! and a stamp claiming otherwise is the `market_bars_v2` failure again.
//!
//! # IV, never greeks (INV-07)
//!
//! Tradier's chain endpoint returns a full greeks block when asked. Every greek in
//! it is **dropped here, at the parser**, and only the implied volatility survives,
//! together with the model that produced it. A greek is a function of inputs plus a
//! model; storing one makes a number nobody can reproduce once either changes.
//!
//! Tradier does not publish the rate and dividend assumptions behind its own IV, so
//! `iv_rate` and `iv_div` are `None` rather than a plausible guess. `iv_model`
//! records whose number this is, so a reader can tell "Tradier's IV" from "IV we
//! computed" instead of discovering the difference in a backtest.
//!
//! # The universe gate, and what happens outside it
//!
//! ADR-P2-26's `universe_id = 1` is `|moneyness − 1| ≤ 0.30`, `dte ∈ [1, 400]`,
//! two-sided quote. Contracts outside it are **not dropped** — they are collected
//! at the end-of-day slot instead, tagged `snapshot_cadence = "daily"`. Deleting
//! them would be an irreversible decision taken on a forward-only series to save
//! disk, which is the trade this whole phase exists to refuse.

use chrono::{DateTime, Duration, NaiveDate, Timelike, Utc};
use dataplane::options::UniverseGate;
use dataplane::source::{Collection, Knowledge, SourceDecl};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::ratelimit::{Denied, RateLimiter, VenueLimit};
use crate::secret::{Credential, CredentialOrigin};

/// The venue name, as it appears in `dataplane.venue` and in the stores.
pub const VENUE: &str = "tradier";

/// Default snapshot cadence for in-universe contracts (P-0.6: `cadence_secs`
/// for `equity.option_chains`).
pub const CADENCE_SECS: i64 = 3600;

/// The UTC hour whose slot carries the end-of-day sweep of out-of-universe
/// contracts. 20:00 UTC is the US equity close (21:00 in winter); the exact hour
/// is not load-bearing, only that exactly one slot a day carries the sweep, so
/// out-of-gate contracts are collected daily rather than hourly or never.
pub const DAILY_SLOT_HOUR_UTC: u32 = 20;

const CHAINS_URL: &str = "https://api.tradier.com/v1/markets/options/chains";
const EXPIRATIONS_URL: &str = "https://api.tradier.com/v1/markets/options/expirations";
const QUOTES_URL: &str = "https://api.tradier.com/v1/markets/quotes";

/// Identify the platform rather than pretending to be a browser, so a venue can
/// throttle us specifically instead of our whole egress IP.
const USER_AGENT: &str = concat!(
    "trading_bot-research/",
    env!("CARGO_PKG_VERSION"),
    " (+forward data collector)"
);

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("rate limit: {0}")]
    RateLimited(#[from] Denied),
    #[error("http: {0}")]
    Http(String),
    #[error("tradier returned an error body: {0}")]
    Venue(String),
    #[error("unparseable field {field}: {value:?}")]
    Unparseable { field: &'static str, value: String },
    /// A row could not state whether its knowledge time was observed or derived.
    /// It is not written (Set P §5).
    #[error("provenance: {0}")]
    Provenance(String),
    /// Moneyness decides the universe gate and is `NOT NULL` in the table. Without
    /// the underlying's price there is nothing to compute it from, and a gate
    /// decided on a guessed spot is worse than no snapshot.
    #[error("no underlying price for {0}: moneyness cannot be computed, so the snapshot is refused rather than gated on a guess")]
    MissingUnderlyingPrice(String),
    /// The receipt precedes what a source with this declared delay could have
    /// published. Refused rather than stored: this is the shape of the
    /// `market_bars_v2` bug, where history looked observed-live.
    #[error(
        "a snapshot of the {event_time} market was received at {received_at}, sooner than the \
         {declared_lag_secs} s delay Tradier declares: the vendor had not published it yet"
    )]
    SnapshotTooSoon {
        event_time: DateTime<Utc>,
        received_at: DateTime<Utc>,
        declared_lag_secs: i64,
    },
    /// A key in `config/` is a key in the repository (Set P MASTER §5).
    #[error("credential came from {0:?}, not the encrypted credential store")]
    BadCredentialOrigin(CredentialOrigin),
    #[error("credential store returned an empty Tradier token")]
    EmptyToken,
}

/// Which cadence produced a row, mirroring `option_chain_snapshots.snapshot_cadence`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cadence {
    /// In ADR-P2-26's `universe_id = 1`.
    Hourly,
    /// Outside the gate: collected once a day, never dropped.
    Daily,
}

impl Cadence {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hourly => "hourly",
            Self::Daily => "daily",
        }
    }
}

/// How often this job snapshots, and which slot sweeps the tail of the chain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapshotPlan {
    pub cadence_secs: i64,
    pub daily_slot_hour_utc: u32,
    pub gate: UniverseGate,
}

impl Default for SnapshotPlan {
    fn default() -> Self {
        Self {
            cadence_secs: CADENCE_SECS,
            daily_slot_hour_utc: DAILY_SLOT_HOUR_UTC,
            gate: UniverseGate::default(),
        }
    }
}

impl SnapshotPlan {
    /// Whether this slot carries the end-of-day sweep of out-of-gate contracts.
    #[must_use]
    pub fn is_daily_slot(&self, event_time: DateTime<Utc>) -> bool {
        event_time.hour() == self.daily_slot_hour_utc
    }
}

/// One contract in one chain snapshot, ready to store.
///
/// `Serialize` is derived so that
/// [`tests::every_greek_tradier_sends_is_dropped_at_the_parser`] can assert, over
/// the row's own serialized form, that no greek is present. A field added here
/// fails that test.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChainObservation {
    pub venue: String,
    pub underlying: String,
    /// The contract symbol, e.g. `AAPL260116C00200000`.
    pub instrument: String,
    pub expiry: NaiveDate,
    pub strike: Decimal,
    /// `'C'` or `'P'`.
    pub right: char,
    /// The market instant the quotes describe — the cadence slot, never the receipt.
    pub event_time: DateTime<Utc>,
    pub venue_ts: Option<DateTime<Utc>>,
    pub ingest_time: DateTime<Utc>,
    /// Observed or derived — inseparable from the instant itself.
    pub knowledge: Knowledge,
    pub bid: Option<Decimal>,
    pub ask: Option<Decimal>,
    pub mark: Option<Decimal>,
    pub underlying_price: Option<Decimal>,
    pub volume: Option<i64>,
    pub open_interest: Option<i64>,
    /// Implied volatility. The one `f64` here, because IV is a model output rather
    /// than money (INV-05 governs prices; the table agrees).
    pub iv: Option<f64>,
    /// Whose IV this is. Tradier's own, in this collector's case.
    pub iv_model: Option<String>,
    /// Tradier publishes no rate assumption, so this is `None` rather than a guess.
    pub iv_rate: Option<f64>,
    /// Likewise the dividend assumption.
    pub iv_div: Option<f64>,
    pub moneyness: f64,
    pub dte: i32,
    pub in_liquid_universe: bool,
    pub cadence: Cadence,
    /// The `dataplane.source.name` that produced this row. Carried rather than
    /// inferred at the sink, because the name is what selects the declared lag.
    pub source: &'static str,
}

impl ChainObservation {
    /// Idempotence key: P-0.5's `(symbol, expiry, as_of)`.
    ///
    /// A restart, or a second poll inside the same cadence slot, must collapse onto
    /// the same key rather than writing a second row. The table's
    /// `ReplacingMergeTree` ordering agrees.
    #[must_use]
    pub fn dedup_key(&self) -> (String, NaiveDate, i64) {
        (
            self.instrument.clone(),
            self.expiry,
            self.event_time.timestamp_nanos_opt().unwrap_or(0),
        )
    }
}

/// What one chain poll yielded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChainSnapshot {
    pub contracts: Vec<ChainObservation>,
    /// Contracts skipped because a field could not be parsed. Counted rather than
    /// guessed at: a row that cannot state a value does not get written.
    pub skipped: Vec<String>,
    /// Out-of-gate contracts held back for the end-of-day slot. **Deferred, not
    /// dropped** — the number is reported so "we collect the tail daily" stays a
    /// checkable claim rather than a comment.
    pub deferred_to_daily: usize,
}

// ── wire types ────────────────────────────────────────────────────────────────
//
// Prices arrive as unquoted JSON numbers, which `serde_json` would round-trip
// through `f64`. They are held as `RawValue` so the digits that reach `Decimal`
// are the digits Tradier sent (SC-1, INV-05). `iv` is deliberately not: implied
// volatility is a model output and the column is `Float64`.

#[derive(Debug, Deserialize)]
struct ChainEnvelope {
    #[serde(default)]
    options: Option<ChainInner>,
    /// Tradier reports faults and errors out-of-band of a 200 sometimes.
    #[serde(default)]
    fault: Option<serde_json::Value>,
    #[serde(default)]
    errors: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ChainInner {
    #[serde(default)]
    option: Vec<WireOption>,
}

#[derive(Debug, Deserialize)]
struct WireOption {
    symbol: String,
    #[serde(default)]
    underlying: Option<String>,
    strike: Box<RawValue>,
    expiration_date: String,
    /// `"call"` or `"put"`.
    option_type: String,
    #[serde(default)]
    bid: Option<Box<RawValue>>,
    #[serde(default)]
    ask: Option<Box<RawValue>>,
    #[serde(default)]
    last: Option<Box<RawValue>>,
    #[serde(default)]
    volume: Option<i64>,
    #[serde(default)]
    open_interest: Option<i64>,
    /// Epoch milliseconds.
    #[serde(default)]
    bid_date: Option<i64>,
    #[serde(default)]
    ask_date: Option<i64>,
    /// Tradier's greeks block. **Only `mid_iv` is read.** The struct below has no
    /// field for a greek, so there is no path by which one could reach a row even
    /// if a future edit forgot the rule.
    #[serde(default)]
    greeks: Option<WireIv>,
}

/// The only part of Tradier's `greeks` block this platform stores.
///
/// Named `WireIv` rather than `WireGreeks` because what it holds is an implied
/// volatility, and a struct called `WireGreeks` invites someone to add a delta to
/// it.
#[derive(Debug, Deserialize)]
struct WireIv {
    #[serde(default)]
    mid_iv: Option<f64>,
    #[serde(default)]
    smv_vol: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct QuotesEnvelope {
    #[serde(default)]
    quotes: Option<QuotesInner>,
}

#[derive(Debug, Deserialize)]
struct QuotesInner {
    #[serde(default)]
    quote: Option<WireQuote>,
}

#[derive(Debug, Deserialize)]
struct WireQuote {
    #[serde(default)]
    last: Option<Box<RawValue>>,
    #[serde(default)]
    prevclose: Option<Box<RawValue>>,
}

#[derive(Debug, Deserialize)]
struct ExpirationsEnvelope {
    #[serde(default)]
    expirations: Option<ExpirationsInner>,
}

#[derive(Debug, Deserialize)]
struct ExpirationsInner {
    #[serde(default)]
    date: Vec<String>,
}

/// A venue number as its own digits, then as a `Decimal`.
///
/// `RawValue::get()` hands back exactly the bytes Tradier sent. Nothing here
/// becomes an `f64`, which is the only reason this function exists rather than
/// `n.as_f64()`.
fn dec(field: &'static str, v: &RawValue) -> Result<Decimal, ChainError> {
    let s = v.get().trim();
    let s = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s);
    if s.is_empty() || s == "null" {
        return Err(ChainError::Unparseable {
            field,
            value: s.to_string(),
        });
    }
    s.parse::<Decimal>().map_err(|_| ChainError::Unparseable {
        field,
        value: s.to_string(),
    })
}

fn opt_dec(field: &'static str, v: Option<&RawValue>) -> Option<Decimal> {
    v.and_then(|raw| dec(field, raw).ok())
}

fn ms_to_utc(ms: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(ms)
}

/// Round an instant down to the cadence grid, after undoing the vendor's declared
/// delay.
///
/// This is the row's `event_time`, and it is what makes the snapshot idempotent:
/// every poll inside one slot describes the *same* market instant, so two polls in
/// an hour collapse to one row instead of becoming two market states that never
/// existed.
///
/// Undoing the delay first is what keeps the stamp honest. A poll at 14:15 holding
/// 15-minute-delayed quotes is looking at the 14:00 market, and saying so is the
/// difference between a usable row and fifteen minutes of lookahead.
#[must_use]
pub fn snapshot_slot(
    received_at: DateTime<Utc>,
    cadence_secs: i64,
    declared_lag: std::time::Duration,
) -> DateTime<Utc> {
    let lag = Duration::from_std(declared_lag).unwrap_or_else(|_| Duration::zero());
    let shifted = received_at - lag;
    let s = shifted.timestamp();
    let cadence = cadence_secs.max(1);
    DateTime::from_timestamp(s - s.rem_euclid(cadence), 0).unwrap_or(shifted)
}

// ── parsing (pure, and therefore testable without a network or a credential) ──

/// Parse a `/markets/options/chains` response into snapshot rows for the cadence
/// slot the receipt falls in.
///
/// # Errors
/// A venue error body, malformed JSON, or a missing underlying price.
pub fn parse_chain(
    body: &str,
    underlying: &str,
    underlying_price: Option<Decimal>,
    received_at: DateTime<Utc>,
    plan: &SnapshotPlan,
) -> Result<ChainSnapshot, ChainError> {
    let src = SourceDecl::tradier_chain();
    let event_time = snapshot_slot(received_at, plan.cadence_secs, src.declared_lag);
    snapshot_at(
        body,
        underlying,
        underlying_price,
        event_time,
        received_at,
        plan,
    )
}

/// Parse a chain response into rows for an **explicitly named** cadence slot.
///
/// The job's retry path names the slot it is re-collecting rather than inferring
/// one from the moment the retry happened, so this is the entry point that has to
/// carry the guard: a receipt that precedes `event_time + the declared lag` is
/// refused, because a vendor that declares a 900 s delay cannot have published the
/// 14:00 market at 14:05, and a row saying it did is lookahead however it got
/// there.
///
/// # Errors
/// [`ChainError::SnapshotTooSoon`] for an impossible receipt, plus everything
/// [`parse_chain`] can return.
pub fn snapshot_at(
    body: &str,
    underlying: &str,
    underlying_price: Option<Decimal>,
    event_time: DateTime<Utc>,
    received_at: DateTime<Utc>,
    plan: &SnapshotPlan,
) -> Result<ChainSnapshot, ChainError> {
    let src = SourceDecl::tradier_chain();
    let lag = Duration::from_std(src.declared_lag).unwrap_or_else(|_| Duration::zero());
    if received_at < event_time + lag {
        return Err(ChainError::SnapshotTooSoon {
            event_time,
            received_at,
            declared_lag_secs: lag.num_seconds(),
        });
    }

    let env: ChainEnvelope =
        serde_json::from_str(body).map_err(|e| ChainError::Http(e.to_string()))?;
    if let Some(f) = env.fault.or(env.errors) {
        return Err(ChainError::Venue(f.to_string()));
    }
    let spot = underlying_price
        .ok_or_else(|| ChainError::MissingUnderlyingPrice(underlying.to_string()))?;
    let spot_f = spot
        .to_f64()
        .filter(|s| *s > 0.0)
        .ok_or_else(|| ChainError::MissingUnderlyingPrice(underlying.to_string()))?;

    // The receipt is the knowledge time, and it is observed. Built once, outside
    // the loop, so there is no per-contract branch that could stamp one row
    // differently from its neighbour.
    let knowledge = src
        .knowledge_for(event_time, Collection::Forward { received_at })
        .map_err(|e| ChainError::Provenance(e.to_string()))?;

    let daily_slot = plan.is_daily_slot(event_time);
    let mut out = ChainSnapshot::default();

    for o in env.options.map(|i| i.option).unwrap_or_default() {
        let Ok(strike) = dec("strike", &o.strike) else {
            out.skipped.push(o.symbol);
            continue;
        };
        let Ok(expiry) = NaiveDate::parse_from_str(&o.expiration_date, "%Y-%m-%d") else {
            out.skipped.push(o.symbol);
            continue;
        };
        let right = match o.option_type.as_str() {
            "call" => 'C',
            "put" => 'P',
            // Not defaulted to a call: a contract whose side is unknown is worse
            // than no contract, because its sign is wrong in every study.
            _ => {
                out.skipped.push(o.symbol);
                continue;
            }
        };
        let Some(strike_f) = strike.to_f64().filter(|k| *k > 0.0) else {
            out.skipped.push(o.symbol);
            continue;
        };

        let bid = opt_dec("bid", o.bid.as_deref());
        let ask = opt_dec("ask", o.ask.as_deref());
        // `mark` is the midpoint when both sides quote, and Tradier's last trade
        // otherwise. It is never invented from one side of the book.
        let mark = match (bid, ask) {
            (Some(b), Some(a)) if b > Decimal::ZERO && a > Decimal::ZERO => {
                // Normalized because `Decimal` division returns a scale decided by
                // the division algorithm, not by the inputs: the midpoint of two
                // 17-digit premia comes back with a trailing zero the money does
                // not have. Left alone, two rows holding the same mark would
                // stringify differently depending on the scale of the quotes that
                // produced them, and these rows are compared and round-tripped.
                // The value is unchanged; only its scale is made canonical.
                Some(((b + a) / Decimal::from(2)).normalize())
            }
            _ => opt_dec("last", o.last.as_deref()),
        };

        // Strike over spot, so the gate `|moneyness − 1| ≤ 0.30` is symmetric about
        // at-the-money in strike space. Documented rather than assumed, because the
        // opposite convention silently reinterprets ADR-P2-26's threshold.
        let moneyness = strike_f / spot_f;
        let dte = i32::try_from((expiry - event_time.date_naive()).num_days()).unwrap_or(i32::MAX);

        // Only the implied volatility survives the greeks block. `mid_iv` first,
        // `smv_vol` as Tradier's own fallback; either way `iv_model` records whose
        // number it is, and `iv_rate`/`iv_div` stay `None` because Tradier does not
        // publish them.
        let (iv, iv_model) = match o.greeks.as_ref() {
            Some(g) => match (g.mid_iv, g.smv_vol) {
                (Some(iv), _) if iv > 0.0 => (Some(iv), Some("tradier_mid_iv".to_string())),
                (_, Some(iv)) if iv > 0.0 => (Some(iv), Some("tradier_smv_vol".to_string())),
                _ => (None, None),
            },
            None => (None, None),
        };

        let candidate = GateInputs {
            bid,
            ask,
            moneyness,
            dte,
        };
        let in_universe = candidate.admits(&plan.gate);
        if !in_universe && !daily_slot {
            // Deferred to the end-of-day slot, never deleted (ADR-P2-26).
            out.deferred_to_daily += 1;
            continue;
        }

        let venue_ts = o
            .ask_date
            .into_iter()
            .chain(o.bid_date)
            .filter(|ms| *ms > 0)
            .max()
            .and_then(ms_to_utc);

        out.contracts.push(ChainObservation {
            venue: VENUE.into(),
            underlying: o.underlying.unwrap_or_else(|| underlying.to_string()),
            instrument: o.symbol,
            expiry,
            strike,
            right,
            event_time,
            venue_ts,
            ingest_time: received_at,
            knowledge,
            bid,
            ask,
            mark,
            underlying_price: Some(spot),
            volume: o.volume,
            open_interest: o.open_interest,
            iv,
            iv_model,
            iv_rate: None,
            iv_div: None,
            moneyness,
            dte,
            in_liquid_universe: in_universe,
            cadence: if in_universe {
                Cadence::Hourly
            } else {
                Cadence::Daily
            },
            source: src.name,
        });
    }
    Ok(out)
}

/// The gate's inputs for one contract, so the decision is made against
/// [`UniverseGate`] rather than re-implemented here.
struct GateInputs {
    bid: Option<Decimal>,
    ask: Option<Decimal>,
    moneyness: f64,
    dte: i32,
}

impl GateInputs {
    fn admits(&self, gate: &UniverseGate) -> bool {
        let quoted = self.bid.is_some_and(|b| b > Decimal::ZERO)
            && self.ask.is_some_and(|a| a > Decimal::ZERO);
        (self.moneyness - 1.0).abs() <= gate.max_abs_log_moneyness_dev
            && (gate.min_dte..=gate.max_dte).contains(&self.dte)
            && (!gate.require_two_sided_quote || quoted)
    }
}

/// Parse a `/markets/quotes` response for the underlying's price.
///
/// # Errors
/// Malformed JSON. A quote with no usable price is `Ok(None)`, so the caller can
/// refuse the snapshot with the symbol in the message rather than guessing a spot.
pub fn parse_underlying_price(body: &str) -> Result<Option<Decimal>, ChainError> {
    let env: QuotesEnvelope =
        serde_json::from_str(body).map_err(|e| ChainError::Http(e.to_string()))?;
    let Some(q) = env.quotes.and_then(|i| i.quote) else {
        return Ok(None);
    };
    Ok(opt_dec("last", q.last.as_deref())
        .or_else(|| opt_dec("prevclose", q.prevclose.as_deref()))
        .filter(|p| *p > Decimal::ZERO))
}

/// Parse a `/markets/options/expirations` response.
///
/// # Errors
/// Malformed JSON.
pub fn parse_expirations(body: &str) -> Result<Vec<NaiveDate>, ChainError> {
    let env: ExpirationsEnvelope =
        serde_json::from_str(body).map_err(|e| ChainError::Http(e.to_string()))?;
    Ok(env
        .expirations
        .map(|i| i.date)
        .unwrap_or_default()
        .iter()
        .filter_map(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .collect())
}

// ── the client ────────────────────────────────────────────────────────────────

/// A rate-limited reader of Tradier's market-data endpoints.
///
/// The limiter **refuses** when the budget is spent; it does not sleep and it does
/// not retry. Tradier publishes its ceiling
/// (<https://documentation.tradier.com/brokerage-api/overview/rate-limiting>), so
/// unlike FRED's this budget is quoted rather than self-imposed — see
/// [`VenueLimit::tradier_market_data`].
///
/// The token comes from the encrypted credential store. A `Credential` whose
/// origin is [`CredentialOrigin::Environment`] cannot build a client, because a
/// key in `config/` is a key in the repository.
pub struct TradierChainClient {
    http: reqwest::Client,
    token: crate::secret::Secret,
    limiter: RateLimiter,
    base_chains: String,
    base_expirations: String,
    base_quotes: String,
}

/// Renders no credential. The token is a [`Secret`](crate::secret::Secret), which
/// redacts in `Debug` — this impl is explicit anyway so a future field cannot be
/// added into a derived `Debug` that prints it.
impl std::fmt::Debug for TradierChainClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TradierChainClient")
            .field("limit", self.limiter.limit())
            .field("token", &self.token)
            .finish_non_exhaustive()
    }
}

impl TradierChainClient {
    /// # Errors
    /// [`ChainError::BadCredentialOrigin`] for a credential that did not come from
    /// the encrypted store, [`ChainError::EmptyToken`] for an empty one, or an HTTP
    /// client that could not be built.
    pub fn new(credential: &Credential) -> Result<Self, ChainError> {
        if credential.origin != CredentialOrigin::CredentialStore {
            return Err(ChainError::BadCredentialOrigin(credential.origin));
        }
        if credential.secret.is_empty() {
            return Err(ChainError::EmptyToken);
        }
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| ChainError::Http(e.to_string()))?;
        Ok(Self {
            http,
            token: credential.secret.clone(),
            limiter: RateLimiter::new(VenueLimit::tradier_market_data()),
            base_chains: CHAINS_URL.to_string(),
            base_expirations: EXPIRATIONS_URL.to_string(),
            base_quotes: QUOTES_URL.to_string(),
        })
    }

    /// Point the client at a different base, for tests against a local stub.
    #[must_use]
    pub fn with_bases(
        mut self,
        chains: impl Into<String>,
        expirations: impl Into<String>,
        quotes: impl Into<String>,
    ) -> Self {
        self.base_chains = chains.into();
        self.base_expirations = expirations.into();
        self.base_quotes = quotes.into();
        self
    }

    /// Install a different declared limit. Tightening is always safe; the venue's
    /// published limit is the default, and the sandbox's is half of it.
    #[must_use]
    pub fn with_limit(mut self, limit: VenueLimit) -> Self {
        self.limiter = RateLimiter::new(limit);
        self
    }

    #[must_use]
    pub const fn limiter(&self) -> &RateLimiter {
        &self.limiter
    }

    async fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<String, ChainError> {
        self.limiter.try_acquire()?;
        let resp = self
            .http
            .get(url)
            .query(query)
            .bearer_auth(self.token.expose())
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| ChainError::Http(e.to_string()))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| ChainError::Http(e.to_string()))?;
        if !status.is_success() {
            // Truncated: an upstream error body can echo the request, and the
            // request carries the token.
            return Err(ChainError::Http(format!(
                "{status}: {}",
                body.chars().take(200).collect::<String>()
            )));
        }
        Ok(body)
    }

    /// The expirations Tradier lists for an underlying.
    ///
    /// # Errors
    /// Refused by the limiter, an HTTP failure, or a venue error.
    pub async fn expirations(&self, underlying: &str) -> Result<Vec<NaiveDate>, ChainError> {
        let body = self
            .get(&self.base_expirations, &[("symbol", underlying)])
            .await?;
        parse_expirations(&body)
    }

    /// The underlying's price, for moneyness.
    ///
    /// # Errors
    /// Refused by the limiter, an HTTP failure, or a venue error.
    pub async fn underlying_price(&self, underlying: &str) -> Result<Option<Decimal>, ChainError> {
        let body = self
            .get(&self.base_quotes, &[("symbols", underlying)])
            .await?;
        parse_underlying_price(&body)
    }

    /// Snapshot one expiry's chain.
    ///
    /// # Errors
    /// Refused by the limiter, an HTTP failure, a venue error, or a missing
    /// underlying price.
    pub async fn chain(
        &self,
        underlying: &str,
        expiry: NaiveDate,
        underlying_price: Option<Decimal>,
        plan: &SnapshotPlan,
    ) -> Result<ChainSnapshot, ChainError> {
        let body = self
            .get(
                &self.base_chains,
                &[
                    ("symbol", underlying),
                    ("expiration", &expiry.format("%Y-%m-%d").to_string()),
                    // Asking for greeks is how the IV arrives. Every greek in the
                    // response is dropped at the parser (INV-07).
                    ("greeks", "true"),
                ],
            )
            .await?;
        parse_chain(&body, underlying, underlying_price, Utc::now(), plan)
    }
}

// ── the sink ──────────────────────────────────────────────────────────────────

/// Where a chain snapshot's rows go.
///
/// A trait rather than a direct call for the same reason
/// [`crate::derivs::sink::ForwardSink`] is one: the only way to prove "a snapshot
/// writes what it collected, once" is to count, and you cannot count through a
/// database. `SourceIds`, `SinkError` and `Beat` are reused from that module
/// unchanged — the source catalogue and the heartbeat are properties of forward
/// collection, not of any one series.
#[async_trait::async_trait]
pub trait ChainSink: Send + Sync {
    /// Append snapshot rows. Returns how many were handed to the store.
    ///
    /// # Errors
    /// Implementation-defined; see [`crate::derivs::sink::SinkError`].
    async fn chain(
        &self,
        rows: &[ChainObservation],
    ) -> Result<usize, crate::derivs::sink::SinkError>;

    /// Record that the series produced rows, so the silence alarm has somewhere
    /// cheap to look (P-0.6 item 2).
    ///
    /// # Errors
    /// Implementation-defined.
    async fn heartbeat(
        &self,
        beats: &[crate::derivs::sink::Beat],
    ) -> Result<usize, crate::derivs::sink::SinkError>;
}

/// The sink that writes ClickHouse — the one that matters.
pub struct ClickHouseChainSink {
    url: String,
    sources: crate::derivs::sink::SourceIds,
}

impl ClickHouseChainSink {
    /// `url` is a full ClickHouse URL **with the database in it**
    /// (`http://user:pass@host:8123/trading`). The database is load-bearing: the
    /// `clickhouse` crate connects to `default` when it is omitted, which is how a
    /// previous investigation concluded these tables did not exist.
    #[must_use]
    pub fn new(url: impl Into<String>, sources: crate::derivs::sink::SourceIds) -> Self {
        Self {
            url: url.into(),
            sources,
        }
    }
}

#[async_trait::async_trait]
impl ChainSink for ClickHouseChainSink {
    async fn chain(
        &self,
        rows: &[ChainObservation],
    ) -> Result<usize, crate::derivs::sink::SinkError> {
        use storage::clickhouse::forward as store;
        if rows.is_empty() {
            return Ok(0);
        }
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push(store::ChainSnapshotRow {
                venue: r.venue.clone(),
                underlying: r.underlying.clone(),
                instrument: r.instrument.clone(),
                expiry: r.expiry,
                strike: r.strike,
                option_right: r.right,
                event_time: r.event_time,
                venue_ts: r.venue_ts,
                ingest_time: r.ingest_time,
                // Passed through, never rebuilt. The store computes quality_flags
                // from it, so provenance cannot be lost here.
                knowledge: r.knowledge,
                bid: r.bid,
                ask: r.ask,
                mark: r.mark,
                underlying_price: r.underlying_price,
                volume: r.volume,
                open_interest: r.open_interest,
                iv: r.iv,
                iv_model: r.iv_model.clone(),
                iv_rate: r.iv_rate,
                iv_div: r.iv_div,
                moneyness: r.moneyness,
                dte: r.dte,
                in_liquid_universe: r.in_liquid_universe,
                snapshot_cadence: r.cadence.as_str().to_string(),
                source_id: self.sources.get(r.source)?,
            });
        }
        store::write_chain_snapshot(&self.url, &out)
            .await
            .map_err(|e| crate::derivs::sink::SinkError::Store(e.to_string()))
    }

    async fn heartbeat(
        &self,
        beats: &[crate::derivs::sink::Beat],
    ) -> Result<usize, crate::derivs::sink::SinkError> {
        use storage::clickhouse::forward as store;
        if beats.is_empty() {
            return Ok(0);
        }
        let rows: Vec<store::HeartbeatRow> = beats
            .iter()
            .map(|b| store::HeartbeatRow {
                series: b.series.clone(),
                venue: b.venue.clone(),
                instrument: b.instrument.clone(),
                last_event_time: b.last_event,
                rows_written: b.rows,
            })
            .collect();
        store::write_heartbeats(&self.url, &rows)
            .await
            .map_err(|e| crate::derivs::sink::SinkError::Store(e.to_string()))
    }
}

/// The `forward_series_heartbeat.series` value for this collector.
pub const HEARTBEAT_SERIES: &str = "option_chain_snapshots";

/// Write a snapshot and its heartbeat, in that order.
///
/// The heartbeat goes last and only on success. A heartbeat that beats regardless
/// of the outcome turns the silence alarm into a liveness lie — the alarm would
/// stay quiet while nothing was being stored, which is precisely the failure P-0.6
/// exists to catch.
///
/// # Errors
/// Whatever the sink returns.
pub async fn write_snapshot(
    sink: &dyn ChainSink,
    underlying: &str,
    snapshot: &ChainSnapshot,
) -> Result<usize, crate::derivs::sink::SinkError> {
    let written = sink.chain(&snapshot.contracts).await?;
    if written > 0 {
        if let Some(last) = snapshot.contracts.iter().map(|c| c.event_time).max() {
            sink.heartbeat(&[crate::derivs::sink::Beat {
                series: HEARTBEAT_SERIES.to_string(),
                venue: VENUE.to_string(),
                instrument: underlying.to_string(),
                last_event: last,
                rows: u64::try_from(written).unwrap_or(u64::MAX),
            }])
            .await?;
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::Secret;
    use chrono::TimeZone;
    use dataplane::options::FORBIDDEN_GREEK_COLUMNS;

    /// A real-shaped Tradier chain page, greeks block included, so the test that
    /// asserts no greek is stored has something to drop.
    const CHAIN: &str = r#"{"options":{"option":[
      {"symbol":"AAPL260116C00200000","description":"AAPL Jan 16 2026 $200.00 Call",
       "underlying":"AAPL","strike":200.0,"expiration_date":"2026-01-16",
       "option_type":"call","bid":12.35,"ask":12.55,"last":12.40,
       "volume":1203,"open_interest":45210,
       "bid_date":1768000000000,"ask_date":1768000060000,
       "greeks":{"delta":0.55,"gamma":0.012,"theta":-0.08,"vega":0.34,"rho":0.21,
                 "phi":-0.01,"bid_iv":0.271,"mid_iv":0.2734,"ask_iv":0.276,
                 "smv_vol":0.2701,"updated_at":"2026-01-02 15:00:00"}},
      {"symbol":"AAPL260116P00200000","description":"AAPL Jan 16 2026 $200.00 Put",
       "underlying":"AAPL","strike":200.0,"expiration_date":"2026-01-16",
       "option_type":"put","bid":9.10,"ask":9.30,"last":9.20,
       "volume":880,"open_interest":31002,
       "greeks":{"delta":-0.45,"gamma":0.012,"mid_iv":0.2811,"smv_vol":0.2799}},
      {"symbol":"AAPL260116C00500000","description":"AAPL Jan 16 2026 $500.00 Call",
       "underlying":"AAPL","strike":500.0,"expiration_date":"2026-01-16",
       "option_type":"call","bid":0.02,"ask":0.05,"last":0.03,
       "volume":1,"open_interest":12,
       "greeks":{"delta":0.01,"mid_iv":0.61}}
    ]}}"#;

    fn spot() -> Decimal {
        Decimal::new(20000, 2) // 200.00
    }

    fn plan() -> SnapshotPlan {
        SnapshotPlan::default()
    }

    /// 2026-01-02, hour `h`, minute `m`, UTC.
    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 2, h, m, 0).unwrap()
    }

    fn punctual() -> DateTime<Utc> {
        at(14, 15) // the 14:00 slot, plus the declared 900 s
    }

    fn snap(received_at: DateTime<Utc>) -> ChainSnapshot {
        parse_chain(CHAIN, "AAPL", Some(spot()), received_at, &plan()).expect("a chain parses")
    }

    // ── P-0.5 acceptance clause 1 ─────────────────────────────────────────────

    /// The acceptance clause as arithmetic: `knowledge_time` is the observation,
    /// and the observation sits exactly the declared 900 s after the market instant
    /// the row describes.
    ///
    /// Both halves matter. If `knowledge_time` were the *slot*, the row would claim
    /// to have been knowable before Tradier published it. If `event_time` were the
    /// *receipt*, the row would claim the 14:15 market when Tradier had shown us
    /// 14:00 — fifteen minutes of lookahead in every study that reads it.
    #[test]
    fn a_snapshots_knowledge_time_is_its_observation_nine_hundred_seconds_after_the_market_it_describes(
    ) {
        let received = punctual();
        let s = snap(received);
        assert!(!s.contracts.is_empty());
        for c in &s.contracts {
            assert_eq!(
                c.event_time,
                at(14, 0),
                "the row describes the 14:00 market"
            );
            assert_eq!(
                c.knowledge.time(),
                received,
                "knowledge_time is the receipt, not a sentinel"
            );
            assert_eq!(
                c.knowledge.time(),
                c.event_time + Duration::seconds(900),
                "a punctual poll knows the market exactly one declared lag later"
            );
            assert_eq!(
                c.knowledge.implied_lag(c.event_time),
                std::time::Duration::from_secs(900),
                "and that lag is what a dataset carries into EmbargoInputs"
            );
        }
    }

    /// Observed, but observed *late*: the row is forward evidence (Set P D-P3) and
    /// carries no backfill sentinel, while still declaring a 900 s lag. A collector
    /// that marked these derived would run for months and leave Gate 15 empty.
    #[test]
    fn the_row_is_observed_rather_than_backfilled_and_still_declares_the_delay() {
        for c in &snap(punctual()).contracts {
            assert!(!c.knowledge.is_derived(), "the receipt is real");
            assert!(c.knowledge.is_forward_evidence());
            assert_eq!(c.knowledge.quality_flags(), dataplane::QualityFlags::NONE);
        }
        assert_eq!(
            SourceDecl::tradier_chain().declared_lag,
            std::time::Duration::from_secs(900)
        );
    }

    /// The second half of clause 1: *the source row records that lag*. The row must
    /// name the source that migration 0098 created for chain snapshots, not
    /// migration 0046's `tradier_rest` — that one is the delayed-quote collector,
    /// a different fact about the same venue.
    #[test]
    fn the_row_names_the_source_whose_catalogue_entry_records_the_lag() {
        let src = SourceDecl::tradier_chain();
        assert_eq!(src.name, "tradier_chain_rest");
        for c in &snap(punctual()).contracts {
            assert_eq!(c.source, src.name);
        }
        // And the sink resolves that name to migration 0098's id rather than
        // defaulting it.
        let cat = crate::derivs::sink::SourceIds::from_pairs(&[("tradier_chain_rest", 19)]);
        assert_eq!(cat.get(src.name).unwrap(), 19);
        assert!(crate::derivs::sink::SourceIds::default()
            .get(src.name)
            .is_err());
    }

    /// Adversarial: a receipt sooner than the declared delay allows is refused.
    /// A vendor that documents a 15-minute delay cannot have published the 14:00
    /// market at 14:05, and a row that says it did is the `market_bars_v2` bug
    /// wearing a different hat.
    #[test]
    fn a_snapshot_received_sooner_than_the_declared_delay_is_refused() {
        let e = snapshot_at(CHAIN, "AAPL", Some(spot()), at(14, 0), at(14, 5), &plan())
            .expect_err("a receipt inside the declared delay must be refused");
        match e {
            ChainError::SnapshotTooSoon {
                declared_lag_secs, ..
            } => assert_eq!(declared_lag_secs, 900),
            other => panic!("{other:?}"),
        }
        // Exactly one lag later is the boundary, and it is admitted.
        assert!(snapshot_at(CHAIN, "AAPL", Some(spot()), at(14, 0), at(14, 15), &plan()).is_ok());
    }

    // ── P-0.5 acceptance clause 2: idempotence per (symbol, expiry, as_of) ────

    /// Two polls inside one cadence slot describe the *same* market instant and
    /// must collapse onto one key. Stamping the receipt as the event time would
    /// turn one hourly snapshot into sixty market states that never existed.
    #[test]
    fn two_polls_in_one_cadence_slot_are_one_snapshot() {
        let keys = |t| {
            let mut k: Vec<_> = snap(t)
                .contracts
                .iter()
                .map(ChainObservation::dedup_key)
                .collect();
            k.sort();
            k
        };
        let a = keys(at(14, 15));
        let b = keys(at(15, 10)); // still the 14:00 slot once the delay is undone
        assert_eq!(a, b, "two polls in one slot must produce the same keys");
        assert!(!a.is_empty());
        // And the next slot is genuinely a different market instant.
        assert_ne!(a, keys(at(15, 20)));
    }

    /// The key is the acceptance's own tuple, and each component separates rows
    /// that must not collapse: a call and a put on the same strike and expiry are
    /// two contracts, not one.
    #[test]
    fn the_idempotence_key_is_symbol_expiry_and_as_of() {
        let s = snap(punctual());
        let call = s
            .contracts
            .iter()
            .find(|c| c.right == 'C' && c.strike == spot())
            .expect("the 200 call");
        let put = s
            .contracts
            .iter()
            .find(|c| c.right == 'P')
            .expect("the 200 put");
        assert_eq!(call.expiry, put.expiry);
        assert_eq!(call.strike, put.strike);
        assert_ne!(call.dedup_key(), put.dedup_key());
        assert_eq!(call.dedup_key().1, call.expiry);
        assert_eq!(
            call.dedup_key().2,
            call.event_time.timestamp_nanos_opt().unwrap()
        );
    }

    // ── INV-07: IV is stored, greeks are not ──────────────────────────────────

    /// Tradier hands us a full greeks block. Not one of those numbers reaches a
    /// row — asserted over the row's own serialized form, so a field added to
    /// [`ChainObservation`] fails here rather than in a migration review.
    #[test]
    fn every_greek_tradier_sends_is_dropped_at_the_parser() {
        assert!(
            CHAIN.contains("\"delta\"") && CHAIN.contains("\"vega\""),
            "the fixture must actually carry greeks, or this test proves nothing"
        );
        let s = snap(punctual());
        for c in &s.contracts {
            let json = serde_json::to_value(c).unwrap();
            let obj = json.as_object().expect("an object");
            for g in FORBIDDEN_GREEK_COLUMNS {
                assert!(
                    obj.keys().all(|k| k != g),
                    "{g} must never reach a stored snapshot row (INV-07)"
                );
            }
            // Nor smuggled in as a nested blob.
            let flat = serde_json::to_string(c).unwrap();
            for g in FORBIDDEN_GREEK_COLUMNS {
                assert!(!flat.contains(&format!("\"{g}\"")), "{g} in {flat}");
            }
        }
        // The IV *is* kept — dropping greeks must not mean dropping the one number
        // the table exists to hold.
        assert!(s.contracts.iter().all(|c| c.iv.is_some()));
    }

    /// Whose IV it is, recorded; and the inputs Tradier does not publish are left
    /// `None` rather than invented. A plausible-looking `iv_rate` nobody sourced is
    /// how a "reproducible" greek becomes unreproducible.
    #[test]
    fn tradiers_iv_is_recorded_as_tradiers_and_its_model_inputs_are_not_invented() {
        let s = snap(punctual());
        let c = s
            .contracts
            .iter()
            .find(|c| c.right == 'C' && c.strike == spot())
            .unwrap();
        assert_eq!(c.iv, Some(0.2734), "mid_iv, not bid_iv and not ask_iv");
        assert_eq!(c.iv_model.as_deref(), Some("tradier_mid_iv"));
        assert_eq!(c.iv_rate, None);
        assert_eq!(c.iv_div, None);
        // The underlying price it was measured against is kept, because IV without
        // its spot cannot be checked after the fact.
        assert_eq!(c.underlying_price, Some(spot()));
    }

    #[test]
    fn a_missing_mid_iv_falls_back_to_tradiers_own_smv_and_says_which() {
        let body = r#"{"options":{"option":[
          {"symbol":"AAPL260116C00200000","underlying":"AAPL","strike":200.0,
           "expiration_date":"2026-01-16","option_type":"call","bid":1.0,"ask":1.1,
           "greeks":{"delta":0.5,"smv_vol":0.31}}]}}"#;
        let s = parse_chain(body, "AAPL", Some(spot()), punctual(), &plan()).unwrap();
        assert_eq!(s.contracts[0].iv, Some(0.31));
        assert_eq!(s.contracts[0].iv_model.as_deref(), Some("tradier_smv_vol"));
    }

    // ── ADR-P2-26: the universe gate, and the tail that is never dropped ──────

    /// Out-of-gate contracts are held for the end-of-day slot, not discarded. The
    /// count is reported, so "we collect the tail daily" is checkable instead of
    /// being a comment above a `continue`.
    #[test]
    fn a_contract_outside_the_universe_is_deferred_to_the_daily_slot_not_dropped() {
        let hourly = snap(punctual()); // 14:00 slot — not the daily one
        assert!(hourly.contracts.iter().all(|c| c.in_liquid_universe));
        assert!(hourly
            .contracts
            .iter()
            .all(|c| c.cadence == Cadence::Hourly));
        assert_eq!(
            hourly.deferred_to_daily, 1,
            "the $500 strike is 2.5x spot and is outside |moneyness - 1| <= 0.30"
        );

        // The daily slot sweeps it up, tagged as a daily row.
        let daily = snap(at(20, 15));
        assert_eq!(daily.deferred_to_daily, 0);
        let tail = daily
            .contracts
            .iter()
            .find(|c| !c.in_liquid_universe)
            .expect("the out-of-gate contract is collected at end of day");
        assert_eq!(tail.cadence, Cadence::Daily);
        assert_eq!(tail.strike, Decimal::from(500));
        assert_eq!(
            daily.contracts.len(),
            hourly.contracts.len() + 1,
            "the daily slot is a superset, never a replacement"
        );
    }

    /// The gate is ADR-P2-26's, evaluated against [`UniverseGate`] rather than
    /// re-implemented, and the convention is stated: moneyness is strike over spot.
    #[test]
    fn moneyness_is_strike_over_spot_and_the_gate_is_adr_p2_26s() {
        let g = UniverseGate::default();
        assert_eq!(g.max_abs_log_moneyness_dev, 0.30);
        assert_eq!((g.min_dte, g.max_dte), (1, 400));
        assert!(g.require_two_sided_quote);

        let s = snap(at(20, 15));
        let atm = s.contracts.iter().find(|c| c.strike == spot()).unwrap();
        assert!((atm.moneyness - 1.0).abs() < 1e-12, "200 / 200 = 1");
        let far = s.contracts.iter().find(|c| !c.in_liquid_universe).unwrap();
        assert!((far.moneyness - 2.5).abs() < 1e-12, "500 / 200 = 2.5");
    }

    /// One-sided quotes fail the gate, so they are collected daily rather than
    /// polluting the hot tier — and still never dropped.
    #[test]
    fn a_one_sided_quote_fails_the_gate_and_is_collected_daily() {
        let body = r#"{"options":{"option":[
          {"symbol":"AAPL260116C00210000","underlying":"AAPL","strike":210.0,
           "expiration_date":"2026-01-16","option_type":"call","bid":0.0,"ask":1.10,
           "greeks":{"mid_iv":0.30}}]}}"#;
        let hourly = parse_chain(body, "AAPL", Some(spot()), punctual(), &plan()).unwrap();
        assert!(hourly.contracts.is_empty());
        assert_eq!(hourly.deferred_to_daily, 1);
        let daily = parse_chain(body, "AAPL", Some(spot()), at(20, 15), &plan()).unwrap();
        assert_eq!(daily.contracts.len(), 1);
        assert!(!daily.contracts[0].in_liquid_universe);
    }

    /// An expiry already past has a non-positive `dte` and cannot be in the liquid
    /// universe — but it is still stored, because a chain that was quoted was a
    /// fact.
    #[test]
    fn an_expired_contract_is_outside_the_gate_and_still_recorded() {
        let body = r#"{"options":{"option":[
          {"symbol":"AAPL251219C00200000","underlying":"AAPL","strike":200.0,
           "expiration_date":"2025-12-19","option_type":"call","bid":1.0,"ask":1.1,
           "greeks":{"mid_iv":0.3}}]}}"#;
        let daily = parse_chain(body, "AAPL", Some(spot()), at(20, 15), &plan()).unwrap();
        assert_eq!(daily.contracts.len(), 1);
        assert!(daily.contracts[0].dte < 0);
        assert!(!daily.contracts[0].in_liquid_universe);
    }

    // ── refusals rather than guesses ──────────────────────────────────────────

    /// Moneyness is `NOT NULL` in the table and it decides the gate. Without the
    /// underlying's price there is nothing to compute it from, and a gate decided
    /// on a guessed spot silently reclassifies the whole chain.
    #[test]
    fn a_chain_without_its_underlying_price_is_refused_rather_than_gated_on_a_guess() {
        let e = parse_chain(CHAIN, "AAPL", None, punctual(), &plan()).unwrap_err();
        assert!(matches!(e, ChainError::MissingUnderlyingPrice(s) if s == "AAPL"));
        let zero =
            parse_chain(CHAIN, "AAPL", Some(Decimal::ZERO), punctual(), &plan()).unwrap_err();
        assert!(matches!(zero, ChainError::MissingUnderlyingPrice(_)));
    }

    /// A contract whose side cannot be read is skipped and counted, never defaulted
    /// to a call. A wrong `option_right` inverts the sign of every study that uses
    /// the row.
    #[test]
    fn a_contract_whose_side_cannot_be_read_is_skipped_and_counted() {
        let body = r#"{"options":{"option":[
          {"symbol":"WEIRD","underlying":"AAPL","strike":200.0,
           "expiration_date":"2026-01-16","option_type":"warrant","bid":1.0,"ask":1.1},
          {"symbol":"BADDATE","underlying":"AAPL","strike":200.0,
           "expiration_date":"not-a-date","option_type":"call","bid":1.0,"ask":1.1}]}}"#;
        let s = parse_chain(body, "AAPL", Some(spot()), punctual(), &plan()).unwrap();
        assert!(s.contracts.is_empty());
        assert_eq!(s.skipped, vec!["WEIRD".to_string(), "BADDATE".to_string()]);
    }

    /// Tradier reports some failures inside a 200 body. A fault is an error, not an
    /// empty chain: treating it as empty is how a silence alarm stays quiet while
    /// nothing is collected.
    #[test]
    fn a_fault_body_is_an_error_not_an_empty_chain() {
        let fault = r#"{"fault":{"faultstring":"Invalid ApiKey","detail":{}}}"#;
        assert!(matches!(
            parse_chain(fault, "AAPL", Some(spot()), punctual(), &plan()),
            Err(ChainError::Venue(_))
        ));
        // An genuinely empty chain is empty, and says so without erroring.
        let empty = r#"{"options":null}"#;
        let s = parse_chain(empty, "AAPL", Some(spot()), punctual(), &plan()).unwrap();
        assert!(s.contracts.is_empty() && s.skipped.is_empty());
    }

    /// Option premia are money, so they keep the digits Tradier sent. `0.1 + 0.2`
    /// is the canonical `f64` failure; here a price is only ever text on its way to
    /// `Decimal` (SC-1, INV-05).
    #[test]
    fn a_premium_keeps_every_digit_it_arrived_with() {
        let body = r#"{"options":{"option":[
          {"symbol":"AAPL260116C00200000","underlying":"AAPL","strike":200.0,
           "expiration_date":"2026-01-16","option_type":"call",
           "bid":0.30000000000000004,"ask":0.30000000000000005,
           "greeks":{"mid_iv":0.3}}]}}"#;
        let s = parse_chain(body, "AAPL", Some(spot()), punctual(), &plan()).unwrap();
        let c = &s.contracts[0];
        assert_eq!(c.bid.unwrap().to_string(), "0.30000000000000004");

        // The hazard, demonstrated on a value that can actually show it. An
        // earlier version of this test asserted that
        // `0.30000000000000004_f64.to_string()` differs from its own literal,
        // which is false: Rust prints the shortest decimal that round-trips, so
        // any `f64` printed back matches the text it was parsed from. The loss
        // shows up one digit further out, where the value no longer fits.
        let too_many_digits = "1.234567890123456789";
        assert_ne!(
            too_many_digits.parse::<f64>().unwrap().to_string(),
            too_many_digits,
            "an f64 cannot hold this many significant digits, which is the whole \
             reason premia reach `Decimal` as text"
        );
        // And the parser does hold them, because a price is a `RawValue` on its
        // way to `Decimal` and never a float.
        let long = format!(
            r#"{{"options":{{"option":[
              {{"symbol":"AAPL260116C00200000","underlying":"AAPL","strike":200.0,
               "expiration_date":"2026-01-16","option_type":"call",
               "bid":{too_many_digits},"ask":{too_many_digits}}}]}}}}"#
        );
        let kept = parse_chain(&long, "AAPL", Some(spot()), punctual(), &plan()).unwrap();
        assert_eq!(
            kept.contracts[0].bid.unwrap().to_string(),
            too_many_digits,
            "every digit Tradier sent"
        );
        // And the mark is the midpoint of the two, not one side of the book.
        assert_eq!(c.mark.unwrap().to_string(), "0.300000000000000045");
    }

    #[test]
    fn a_mark_is_the_midpoint_when_both_sides_quote_and_the_last_trade_otherwise() {
        let s = snap(punctual());
        let c = s.contracts.iter().find(|c| c.right == 'C').unwrap();
        assert_eq!(c.mark, Some(Decimal::new(1245, 2)), "(12.35 + 12.55) / 2");

        let one_sided = r#"{"options":{"option":[
          {"symbol":"X","underlying":"AAPL","strike":200.0,"expiration_date":"2026-01-16",
           "option_type":"call","bid":0.0,"ask":0.0,"last":7.25,"greeks":{"mid_iv":0.3}}]}}"#;
        let d = parse_chain(one_sided, "AAPL", Some(spot()), at(20, 15), &plan()).unwrap();
        assert_eq!(d.contracts[0].mark, Some(Decimal::new(725, 2)));
    }

    #[test]
    fn the_venue_timestamp_is_the_newer_of_the_two_quote_stamps() {
        let s = snap(punctual());
        let c = s.contracts.iter().find(|c| c.right == 'C').unwrap();
        assert_eq!(
            c.venue_ts,
            DateTime::from_timestamp_millis(1_768_000_060_000)
        );
        // Absent stamps are absent, not epoch zero.
        let put = s.contracts.iter().find(|c| c.right == 'P').unwrap();
        assert_eq!(put.venue_ts, None);
    }

    // ── the slot arithmetic ───────────────────────────────────────────────────

    #[test]
    fn the_slot_undoes_the_delay_before_it_floors() {
        let lag = std::time::Duration::from_secs(900);
        assert_eq!(snapshot_slot(at(14, 15), 3600, lag), at(14, 0));
        assert_eq!(snapshot_slot(at(15, 14), 3600, lag), at(14, 0));
        assert_eq!(snapshot_slot(at(15, 15), 3600, lag), at(15, 0));
        // With no declared delay it is a plain hourly floor.
        assert_eq!(
            snapshot_slot(at(14, 15), 3600, std::time::Duration::ZERO),
            at(14, 0)
        );
    }

    #[test]
    fn exactly_one_slot_a_day_carries_the_end_of_day_sweep() {
        let p = plan();
        let sweeps = (0..24).filter(|h| p.is_daily_slot(at(*h, 0))).count();
        assert_eq!(sweeps, 1);
        assert!(p.is_daily_slot(at(DAILY_SLOT_HOUR_UTC, 0)));
    }

    // ── the credential, and the limiter ───────────────────────────────────────

    /// The credential half of Set P MASTER §5: a token that did not come from the
    /// encrypted store cannot construct a client at all.
    #[test]
    fn a_token_not_from_the_credential_store_is_refused() {
        let env = Credential::from_env(Secret::new("tradier-token"));
        assert!(matches!(
            TradierChainClient::new(&env),
            Err(ChainError::BadCredentialOrigin(
                CredentialOrigin::Environment
            ))
        ));
        let empty = Credential::from_store(Secret::new(""));
        assert!(matches!(
            TradierChainClient::new(&empty),
            Err(ChainError::EmptyToken)
        ));
        assert!(TradierChainClient::new(&Credential::from_store(Secret::new("t"))).is_ok());
    }

    /// And the token does not reach a log through the client's own rendering,
    /// which is the ordinary way a key escapes.
    #[test]
    fn the_client_debug_never_prints_the_token() {
        let c = TradierChainClient::new(&Credential::from_store(Secret::new("supersecrettoken")))
            .unwrap();
        let rendered = format!("{c:?}");
        assert!(!rendered.contains("supersecrettoken"), "{rendered}");
        assert!(rendered.contains("***"));
    }

    /// The limiter refuses rather than sleeping, and the budget it spends is
    /// Tradier's published one with the sentence that establishes it.
    #[test]
    fn the_declared_budget_is_tradiers_published_rate_and_it_refuses_above_it() {
        let c = TradierChainClient::new(&Credential::from_store(Secret::new("t"))).unwrap();
        let l = c.limiter().limit();
        assert_eq!(l.venue, "tradier");
        assert_eq!(l.refill_window, std::time::Duration::from_secs(60));
        assert!(
            !l.source_quote.is_empty() && !l.source_quote.contains("self-imposed"),
            "Tradier publishes this one, so it is quoted rather than self-imposed"
        );
        for _ in 0..l.capacity {
            c.limiter().try_acquire().expect("inside the budget");
        }
        assert!(
            c.limiter().try_acquire().is_err(),
            "the request past the declared rate must be refused, not delayed"
        );
        // The sandbox allowance is half of production, and tightening is safe.
        let sandbox = c.with_limit(VenueLimit::tradier_sandbox());
        assert_eq!(sandbox.limiter().limit().capacity, 60);
    }

    // ── scope: this task introduces no corporate-action handling ──────────────

    /// P-0.5's scope note as a build-failing test. The equities schema questions —
    /// corporate actions, INV-03 adjustment factors, surrogate instrument ids —
    /// stay closed behind roadmap gate H-6, and a snapshot job is not the place
    /// they get quietly reopened.
    ///
    /// The check is over this module's own source text, because the property is
    /// "this code does not reach for the adjustment machinery", and that is a fact
    /// about the code rather than about any one value it produces.
    #[test]
    fn no_corporate_action_handling_is_introduced_by_this_task() {
        let src = include_str!("tradier_chain_snapshot.rs");
        // Everything after the marker is this test module quoting the names.
        let code = src
            .split("fn no_corporate_action_handling_is_introduced_by_this_task")
            .next()
            .unwrap();
        for forbidden in [
            "AdjustmentPolicy",
            "factors_at",
            "dataplane::corporate",
            "corp_actions",
            "adjustment_factor",
        ] {
            assert!(
                !code.contains(forbidden),
                "{forbidden} appears in a snapshot collector: corporate actions stay behind gate H-6"
            );
        }
        // Prices are stored as the venue printed them, which is the INV-03 half
        // this task does have to honour: nothing here adjusts anything.
        let s = snap(punctual());
        let c = s.contracts.iter().find(|c| c.right == 'C').unwrap();
        assert_eq!(c.bid, Some(Decimal::new(1235, 2)));
        assert_eq!(c.strike, Decimal::from(200));
    }

    // ── the sink ──────────────────────────────────────────────────────────────

    /// The heartbeat is written only after rows are, so the silence alarm cannot
    /// report liveness for a collector that stored nothing.
    #[tokio::test]
    async fn a_snapshot_that_writes_nothing_leaves_no_heartbeat() {
        #[derive(Default)]
        struct Counting {
            rows: std::sync::atomic::AtomicUsize,
            beats: std::sync::atomic::AtomicUsize,
        }
        #[async_trait::async_trait]
        impl ChainSink for Counting {
            async fn chain(
                &self,
                rows: &[ChainObservation],
            ) -> Result<usize, crate::derivs::sink::SinkError> {
                self.rows
                    .fetch_add(rows.len(), std::sync::atomic::Ordering::Relaxed);
                Ok(rows.len())
            }
            async fn heartbeat(
                &self,
                beats: &[crate::derivs::sink::Beat],
            ) -> Result<usize, crate::derivs::sink::SinkError> {
                self.beats
                    .fetch_add(beats.len(), std::sync::atomic::Ordering::Relaxed);
                Ok(beats.len())
            }
        }
        let sink = Counting::default();
        let empty = ChainSnapshot::default();
        assert_eq!(write_snapshot(&sink, "AAPL", &empty).await.unwrap(), 0);
        assert_eq!(sink.beats.load(std::sync::atomic::Ordering::Relaxed), 0);

        let full = snap(punctual());
        let n = write_snapshot(&sink, "AAPL", &full).await.unwrap();
        assert_eq!(n, full.contracts.len());
        assert_eq!(sink.beats.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    /// An empty batch writes nothing and says so, rather than opening an insert
    /// that commits an empty block. The URL is unreachable on purpose: if an empty
    /// batch talked to the backend this would fail rather than return zero.
    #[tokio::test]
    async fn an_empty_batch_touches_no_backend() {
        let sink = ClickHouseChainSink::new(
            "http://127.0.0.1:1/trading",
            crate::derivs::sink::SourceIds::from_pairs(&[("tradier_chain_rest", 19)]),
        );
        assert_eq!(sink.chain(&[]).await.unwrap(), 0);
        assert_eq!(sink.heartbeat(&[]).await.unwrap(), 0);
    }

    // ── the other two endpoints ───────────────────────────────────────────────

    #[test]
    fn an_underlying_price_falls_back_to_the_previous_close_and_never_to_zero() {
        let last = r#"{"quotes":{"quote":{"symbol":"AAPL","last":201.25,"prevclose":199.5}}}"#;
        assert_eq!(
            parse_underlying_price(last).unwrap(),
            Some(Decimal::new(20125, 2))
        );
        let halted = r#"{"quotes":{"quote":{"symbol":"AAPL","last":null,"prevclose":199.5}}}"#;
        assert_eq!(
            parse_underlying_price(halted).unwrap(),
            Some(Decimal::new(1995, 1))
        );
        let nothing = r#"{"quotes":{"quote":{"symbol":"AAPL","last":0,"prevclose":null}}}"#;
        assert_eq!(parse_underlying_price(nothing).unwrap(), None);
        assert_eq!(parse_underlying_price(r#"{"quotes":null}"#).unwrap(), None);
    }

    #[test]
    fn expirations_parse_and_an_unreadable_date_is_dropped_rather_than_guessed() {
        let body = r#"{"expirations":{"date":["2026-01-16","2026-02-20","nonsense"]}}"#;
        let e = parse_expirations(body).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0], NaiveDate::from_ymd_opt(2026, 1, 16).unwrap());
        assert!(parse_expirations(r#"{"expirations":null}"#)
            .unwrap()
            .is_empty());
    }
}
