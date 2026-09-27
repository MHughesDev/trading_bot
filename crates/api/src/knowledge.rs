//! The knowledge plane's read and write paths (SPEC §5, checklist 3.3/3.5,
//! ADR-P3-02, ADR-P3-05).
//!
//! Two things live here today, and both are the *rule tier*: regime states on
//! volatility terciles, and as-of-bounded neighbour lookup over
//! `knowledge.asset_embedding`. The learned tiers — a filtered HMM/HSMM with
//! BOCPD, a PCA-whitened learned encoder — replace the internals later without
//! changing either signature, which is the point of building the rule tier
//! first: Gate 11 becomes real immediately and the model that replaces it has
//! something to be compared against (ADR-P3-02).
//!
//! ## The two rules that are not conveniences
//!
//! **Filtered only.** [`RegimeStore::filtered_at`] reads
//! `regime_causal.regime_state` and there is no function here that reads
//! `regime_research`. Smoothed probabilities are the answer computed with
//! hindsight; a strategy that sees one earns about 2.2× the Sharpe it will earn
//! live. The database enforces it with a schema grant (AT-42); this module does
//! not offer the call.
//!
//! **As-of bounded.** [`EmbeddingStore::neighbours`] takes an `as_of` and filters
//! `knowledge_time <= as_of`. A neighbour list computed from embeddings the
//! as-of date could not have seen is meta-leakage: the similarity itself carries
//! the future (AT-43).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// A regime label. Statistical, never narrative (ADR-P3-02).
///
/// `high_vol` is a measurement. `risk_off` is a story, and a story the model did
/// not learn — it is the reader's interpretation smuggled into the data, and it
/// survives into every downstream report as though the model had said it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegimeLabel {
    LowVol,
    MidVol,
    HighVol,
}

impl RegimeLabel {
    /// The three, matching `regime_causal.regime_state`'s CHECK.
    pub const ALL: [Self; 3] = [Self::LowVol, Self::MidVol, Self::HighVol];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LowVol => "low_vol",
            Self::MidVol => "mid_vol",
            Self::HighVol => "high_vol",
        }
    }

    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.as_str() == code)
    }

    /// The label for a vol tercile, in the same order
    /// `backtest::gates::evaluators::vol_tercile_regimes` produces them.
    #[must_use]
    pub fn from_tercile(regime: backtest::gates::evaluators::VolRegime) -> Self {
        use backtest::gates::evaluators::VolRegime;
        match regime {
            VolRegime::Low => Self::LowVol,
            VolRegime::Mid => Self::MidVol,
            VolRegime::High => Self::HighVol,
        }
    }

    /// A one-hot filtered distribution.
    ///
    /// The rule tier is deterministic given the window, so its "probability" is
    /// 1 on the observed tercile. That is honest rather than tidy: a rule that
    /// reported 0.7/0.2/0.1 would be inventing an uncertainty it did not
    /// compute, and the learned tier's real distribution would then be
    /// indistinguishable from it in the table.
    #[must_use]
    pub fn one_hot(self) -> Vec<f32> {
        Self::ALL
            .iter()
            .map(|l| if *l == self { 1.0 } else { 0.0 })
            .collect()
    }
}

/// One regime observation.
#[derive(Clone, Debug, PartialEq)]
pub struct RegimeObservation {
    pub event_time: DateTime<Utc>,
    pub label: RegimeLabel,
    /// `P(state | data up to event_time)`. Filtered, always.
    pub p_filtered: Vec<f32>,
    /// When the platform could first have known this. For the rule tier that is
    /// the close of the bar the tercile was computed through — never earlier.
    pub knowledge_time: DateTime<Utc>,
    /// The scope series' log return for this bar — the observable the label was
    /// computed from. `None` for rows written before 0117.
    pub series_log_return: Option<f64>,
}

/// One configured market scope (SPEC §5.4, ADR-P3-07, ADR-P3-08).
///
/// A scope is a *market*, and `series_symbol` is how that market is measured.
/// Keeping the two apart is the whole reason this is a table rather than a list
/// of symbols: "the crypto market" and "BTC-USD" are not the same statement,
/// and the second is the evidence for the first.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RegimeScope {
    pub market_scope: String,
    pub asset_class: String,
    pub series_symbol: String,
    /// `None` defers to the bar reader's best-quality-venue rule.
    pub series_venue: Option<String>,
    /// The stored timeframe read, and the bucket returns are taken over.
    pub series_base_period_secs: i32,
    pub series_period_secs: i32,
    /// Symbols this scope may label. Empty means every instrument of the asset
    /// class — a claim, and one the seeded configuration deliberately does not
    /// make while the traded universe is small.
    pub applies_to: Vec<String>,
    pub rationale: String,
}

impl RegimeScope {
    /// Whether this scope is the market for a strategy trading `symbol` in
    /// `asset_class`.
    ///
    /// The asset class must match — a crypto volatility regime says nothing
    /// about an equity strategy, and labelling one with the other is precisely
    /// the authoritative-looking wrong answer ADR-P3-07 refused to default to.
    #[must_use]
    pub fn covers(&self, asset_class: &str, symbol: Option<&str>) -> bool {
        if !self.asset_class.eq_ignore_ascii_case(asset_class) {
            return false;
        }
        if self.applies_to.is_empty() {
            return true;
        }
        match symbol {
            // An explicit instrument list is a decision about those
            // instruments. Without a symbol there is nothing to check it
            // against, so the scope does not claim the strategy.
            Some(s) => self.applies_to.iter().any(|a| a.eq_ignore_ascii_case(s)),
            None => false,
        }
    }
}

/// Reads `regime_causal.regime_scope`.
///
/// There is no write path here on purpose. Configuring a scope chooses the
/// market a result is measured against, which is a decision an operator makes
/// and an agent must not (SPEC §12.7); the grant says so and this module does
/// not offer the call.
pub struct RegimeScopeStore {
    pg: PgPool,
}

impl RegimeScopeStore {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    /// Every enabled scope, in name order.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn enabled(&self) -> Result<Vec<RegimeScope>, sqlx::Error> {
        let rows: Vec<ScopeRow> = sqlx::query_as(
            "SELECT market_scope, asset_class, series_symbol, series_venue,
                    series_base_period_secs, series_period_secs, applies_to, rationale
               FROM regime_causal.regime_scope
              WHERE enabled
              ORDER BY market_scope",
        )
        .fetch_all(&self.pg)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// The scope that covers a strategy.
    ///
    /// Every outcome but [`ScopeResolution::Scope`] is a real answer the caller
    /// must carry: an unconfigured market has no regimes, and a gate that
    /// invents them from the strategy's own returns has to say so (ADR-P2-29).
    ///
    /// # Errors
    /// Backend failures.
    pub async fn resolve(
        &self,
        asset_class: &str,
        symbol: Option<&str>,
    ) -> Result<ScopeResolution, sqlx::Error> {
        Ok(resolve_from(&self.enabled().await?, asset_class, symbol))
    }
}

/// What asking for a strategy's market scope can answer.
#[derive(Clone, Debug, PartialEq)]
pub enum ScopeResolution {
    Scope(Box<RegimeScope>),
    /// Several scopes claim this asset class and nothing said which market the
    /// strategy trades. Choosing one would be exactly the guess ADR-P3-07
    /// refused to bake in as a default.
    Ambiguous(Vec<String>),
    /// No scope is configured for this market.
    Unconfigured,
}

/// The resolution rule, without a database.
///
/// With a symbol, a scope has to claim that symbol. Without one, a single
/// configured scope for the asset class is unambiguous — there is only one
/// market it could mean — and two or more are not, so the answer is
/// [`ScopeResolution::Ambiguous`] rather than the first row.
#[must_use]
pub fn resolve_from(
    scopes: &[RegimeScope],
    asset_class: &str,
    symbol: Option<&str>,
) -> ScopeResolution {
    if symbol.is_some() {
        return scopes
            .iter()
            .find(|s| s.covers(asset_class, symbol))
            .map_or(ScopeResolution::Unconfigured, |s| {
                ScopeResolution::Scope(Box::new(s.clone()))
            });
    }
    let in_class: Vec<&RegimeScope> = scopes
        .iter()
        .filter(|s| s.asset_class.eq_ignore_ascii_case(asset_class))
        .collect();
    match in_class.len() {
        0 => ScopeResolution::Unconfigured,
        1 => ScopeResolution::Scope(Box::new(in_class[0].clone())),
        _ => ScopeResolution::Ambiguous(in_class.iter().map(|s| s.market_scope.clone()).collect()),
    }
}

/// `regime_causal.regime_scope` as it comes back.
type ScopeRow = (
    String,
    String,
    String,
    Option<String>,
    i32,
    i32,
    Vec<String>,
    String,
);

impl From<ScopeRow> for RegimeScope {
    fn from(r: ScopeRow) -> Self {
        Self {
            market_scope: r.0,
            asset_class: r.1,
            series_symbol: r.2,
            series_venue: r.3,
            series_base_period_secs: r.4,
            series_period_secs: r.5,
            applies_to: r.6,
            rationale: r.7,
        }
    }
}

/// The causal label in force at each of `times`, given a scope's history.
///
/// Two bounds and both are the same idea as [`RegimeStore::filtered_at`]: a row
/// is an answer about `t` only if its event time *and* its knowledge time are at
/// or before `t`. `None` where no row qualifies — an unlabelled observation is
/// dropped by the caller rather than assigned the nearest label, because
/// "nearest" in this direction means the next one, which is the future.
///
/// `history` must be sorted by `event_time` ascending; [`RegimeStore::history`]
/// returns it that way.
#[must_use]
pub fn labels_as_of(
    history: &[RegimeObservation],
    times: &[DateTime<Utc>],
) -> Vec<Option<RegimeLabel>> {
    times
        .iter()
        .map(|t| {
            history
                .iter()
                .rev()
                .find(|o| o.event_time <= *t && o.knowledge_time <= *t)
                .map(|o| o.label)
        })
        .collect()
}

/// Reads and writes `regime_causal.regime_state`.
pub struct RegimeStore {
    pg: PgPool,
}

/// How far behind a window's start to read labels.
///
/// The label in force on the first observation was written at an earlier bar's
/// close, so a history starting at that observation leaves the beginning of every
/// series unlabelled. Sixty days is comfortably more than the labeller's 21-day
/// window needs and costs one bounded read.
///
/// One home, because this was declared privately in `gate_worker` and again in
/// `regime_separation`, and a third copy for the diagnostics path would have made
/// three answers to "how far back do we look" in one crate. Both now read it from
/// here.
pub const LABEL_LOOKBEHIND_DAYS: i64 = 60;

/// The rule tier's model version.
///
/// `model_version` is the fit date for a learned model (ADR-P3-02). The rule
/// tier is not fitted, so it carries a name that says so and cannot be confused
/// with a date: a reader seeing `rule_vol_tercile_v1` in a report knows which
/// tier produced the number.
pub const RULE_MODEL_VERSION: &str = "rule_vol_tercile_v1";

impl RegimeStore {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    /// Label a market's daily returns by volatility tercile and store the result.
    ///
    /// The terciles are taken over the window being labelled, which is a
    /// description of that period rather than a prediction within it — and it is
    /// why `knowledge_time` is the observation's own bar close: a strategy
    /// reading this as-of day *t* gets the label computed from data up to *t*,
    /// even though the tercile boundaries were computed over the whole window.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn write_rule_tier(
        &self,
        market_scope: &str,
        returns: &[(DateTime<Utc>, f64)],
        window: usize,
    ) -> Result<usize, sqlx::Error> {
        let labels = backtest::gates::evaluators::vol_tercile_regimes(returns, window);
        let mut written = 0;
        for ((event_time, series_return), regime) in returns.iter().zip(labels.iter()) {
            let label = RegimeLabel::from_tercile(*regime);
            sqlx::query(
                "INSERT INTO regime_causal.regime_state
                     (market_scope, event_time, model_version, p_filtered, regime_vec,
                      knowledge_time, label, series_log_return)
                 VALUES ($1, $2, $3, $4, $5::real[]::vector, $6, $7, $8)
                 ON CONFLICT (market_scope, event_time, model_version) DO UPDATE
                   -- The label is keyed and does not change; the observable it
                   -- was computed from is backfilled onto rows written before
                   -- 0117 added the column, and only when it is still absent.
                   SET series_log_return = coalesce(
                         regime_state.series_log_return,
                         EXCLUDED.series_log_return)",
            )
            .bind(market_scope)
            .bind(event_time)
            .bind(RULE_MODEL_VERSION)
            .bind(label.one_hot())
            .bind(regime_vec(label))
            .bind(event_time)
            .bind(label.as_str())
            .bind(series_return)
            .execute(&self.pg)
            .await?;
            written += 1;
        }
        Ok(written)
    }

    /// Every filtered state for a scope in `[from, to]`, oldest first.
    ///
    /// The whole history in one read rather than a query per timestamp: the
    /// callers that need this are aligning a strategy's series to the market's,
    /// and N round trips to answer one alignment question is how an analysis job
    /// becomes a load problem.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn history(
        &self,
        market_scope: &str,
        model_version: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<RegimeObservation>, sqlx::Error> {
        let rows: Vec<HistoryRow> = sqlx::query_as(
            "SELECT event_time, label, p_filtered, knowledge_time, series_log_return
               FROM regime_causal.regime_state
              WHERE market_scope = $1 AND model_version = $2
                AND event_time >= $3 AND event_time <= $4
              ORDER BY event_time",
        )
        .bind(market_scope)
        .bind(model_version)
        .bind(from)
        .bind(to)
        .fetch_all(&self.pg)
        .await?;

        Ok(rows
            .into_iter()
            .filter_map(
                |(event_time, label, p_filtered, knowledge_time, series_log_return)| {
                    RegimeLabel::from_code(&label).map(|label| RegimeObservation {
                        event_time,
                        label,
                        p_filtered,
                        knowledge_time,
                        series_log_return,
                    })
                },
            )
            .collect())
    }

    /// The most recent filtered state at or before `as_of`.
    ///
    /// There is no `smoothed_at`. Not "it is not implemented yet" — the smoothed
    /// path is in a schema this process's role cannot reach, and offering the
    /// call would only produce a permission error at a confusing moment.
    ///
    /// # Errors
    /// Backend failures, or a stored label this build does not know.
    pub async fn filtered_at(
        &self,
        market_scope: &str,
        as_of: DateTime<Utc>,
        model_version: &str,
    ) -> Result<Option<RegimeObservation>, sqlx::Error> {
        let row: Option<RegimeRow> = sqlx::query_as(
            "SELECT event_time, label, p_filtered, knowledge_time, series_log_return
             FROM regime_causal.regime_state
             WHERE market_scope = $1 AND model_version = $2
               AND event_time <= $3
               -- The second bound is the one that matters: a row the platform
               -- could not have known at `as_of` is not an answer about `as_of`.
               AND knowledge_time <= $3
             ORDER BY event_time DESC
             LIMIT 1",
        )
        .bind(market_scope)
        .bind(model_version)
        .bind(as_of)
        .fetch_optional(&self.pg)
        .await?;

        Ok(row.and_then(
            |(event_time, label, p_filtered, knowledge_time, series_log_return)| {
                RegimeLabel::from_code(&label).map(|label| RegimeObservation {
                    event_time,
                    label,
                    p_filtered,
                    knowledge_time,
                    series_log_return,
                })
            },
        ))
    }
}

/// `(event_time, label, p_filtered, knowledge_time, series_log_return)` as the
/// row comes back.
type RegimeRow = (DateTime<Utc>, String, Vec<f32>, DateTime<Utc>, Option<f64>);

/// The same shape, for [`RegimeStore::history`].
type HistoryRow = RegimeRow;

/// A 16-dimensional regime coordinate.
///
/// The rule tier has three states, so thirteen of the sixteen are zero. The
/// column is sized for the learned tier rather than the rule one because
/// changing a `vector(n)` later means rewriting every row, and a zero is an
/// honest "this tier does not have that dimension".
fn regime_vec(label: RegimeLabel) -> Vec<f32> {
    let mut v = vec![0.0_f32; 16];
    for (i, l) in RegimeLabel::ALL.iter().enumerate() {
        if *l == label {
            v[i] = 1.0;
        }
    }
    v
}

/// One neighbour of an asset, as of a date.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Neighbour {
    pub instrument_id: i64,
    pub venue_id: i32,
    /// Cosine distance in the whitened 48-d space. Smaller is closer.
    pub distance: f64,
    pub knowledge_time: DateTime<Utc>,
}

/// Exact kNN over `knowledge.asset_embedding`.
pub struct EmbeddingStore {
    pg: PgPool,
}

impl EmbeddingStore {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    /// The `k` nearest assets to one instrument, as of a date.
    ///
    /// Two bounds, and both are the same idea (AT-43): the *query* embedding and
    /// every *candidate* embedding must have been knowable at `as_of`. Bounding
    /// only the candidates would still leak — the vector being searched from
    /// would carry the future, and every neighbour list built on it with it.
    ///
    /// Exact, with no ANN index (ADR-P3-05, R-14). At this scale that is about a
    /// millisecond, and an approximate index's recall depends on its own build
    /// state, which makes the neighbour list something an attacker can probe.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn neighbours(
        &self,
        instrument_id: i64,
        venue_id: i32,
        embedding_version: &str,
        as_of: DateTime<Utc>,
        k: i64,
    ) -> Result<Vec<Neighbour>, sqlx::Error> {
        let rows: Vec<(i64, i32, f64, DateTime<Utc>)> = sqlx::query_as(
            "WITH query AS (
                 SELECT retrieval_vec
                 FROM knowledge.asset_embedding
                 WHERE instrument_id = $1 AND venue_id = $2 AND embedding_version = $3
                   AND knowledge_time <= $4
                 ORDER BY knowledge_time DESC
                 LIMIT 1
             ),
             latest AS (
                 SELECT DISTINCT ON (instrument_id, venue_id)
                        instrument_id, venue_id, retrieval_vec, knowledge_time
                 FROM knowledge.asset_embedding
                 WHERE embedding_version = $3 AND knowledge_time <= $4
                 ORDER BY instrument_id, venue_id, knowledge_time DESC
             )
             SELECT l.instrument_id, l.venue_id,
                    (l.retrieval_vec <=> q.retrieval_vec)::float8 AS distance,
                    l.knowledge_time
             FROM latest l CROSS JOIN query q
             WHERE NOT (l.instrument_id = $1 AND l.venue_id = $2)
             ORDER BY distance
             LIMIT $5",
        )
        .bind(instrument_id)
        .bind(venue_id)
        .bind(embedding_version)
        .bind(as_of)
        .bind(k)
        .fetch_all(&self.pg)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(instrument_id, venue_id, distance, knowledge_time)| Neighbour {
                    instrument_id,
                    venue_id,
                    distance,
                    knowledge_time,
                },
            )
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backtest::gates::evaluators::VolRegime;
    use chrono::TimeZone;

    #[test]
    fn the_labels_are_exactly_the_database_check() {
        let codes: Vec<&str> = RegimeLabel::ALL.iter().map(|l| l.as_str()).collect();
        assert_eq!(codes, vec!["low_vol", "mid_vol", "high_vol"]);
        for l in RegimeLabel::ALL {
            assert_eq!(RegimeLabel::from_code(l.as_str()), Some(l));
        }
        // A narrative label is not a label this model produces.
        assert_eq!(RegimeLabel::from_code("risk_off"), None);
        assert_eq!(RegimeLabel::from_code("crisis"), None);
    }

    #[test]
    fn a_tercile_maps_to_exactly_one_label() {
        assert_eq!(
            RegimeLabel::from_tercile(VolRegime::Low),
            RegimeLabel::LowVol
        );
        assert_eq!(
            RegimeLabel::from_tercile(VolRegime::Mid),
            RegimeLabel::MidVol
        );
        assert_eq!(
            RegimeLabel::from_tercile(VolRegime::High),
            RegimeLabel::HighVol
        );
    }

    /// The rule tier is deterministic, so its distribution is one-hot. Reporting
    /// a softened distribution would invent an uncertainty nothing computed, and
    /// make the rule tier's rows indistinguishable from the learned tier's.
    #[test]
    fn the_rule_tiers_distribution_is_honest_about_being_deterministic() {
        for l in RegimeLabel::ALL {
            let p = l.one_hot();
            assert_eq!(p.len(), 3);
            assert!((p.iter().sum::<f32>() - 1.0).abs() < 1e-6);
            assert_eq!(p.iter().filter(|x| **x == 1.0).count(), 1);
        }
    }

    #[test]
    fn the_regime_vector_is_sized_for_the_learned_tier() {
        for l in RegimeLabel::ALL {
            let v = regime_vec(l);
            assert_eq!(v.len(), 16, "the column is vector(16)");
            assert_eq!(v.iter().filter(|x| **x != 0.0).count(), 1);
        }
    }

    fn scope(applies_to: &[&str]) -> RegimeScope {
        RegimeScope {
            market_scope: "crypto_btc_usd".into(),
            asset_class: "crypto".into(),
            series_symbol: "BTC-USD".into(),
            series_venue: None,
            series_base_period_secs: 60,
            series_period_secs: 86_400,
            applies_to: applies_to.iter().map(ToString::to_string).collect(),
            rationale: "the configured reason".into(),
        }
    }

    /// A crypto volatility regime is not a statement about an equity strategy.
    /// This is ADR-P3-07's whole point expressed as a predicate: the scope has
    /// to refuse a market it does not measure, or configuring one becomes the
    /// default it was refused.
    #[test]
    fn a_scope_does_not_claim_a_market_it_does_not_measure() {
        let s = scope(&["BTC-USD", "ETH-USD"]);
        assert!(s.covers("crypto", Some("BTC-USD")));
        assert!(
            s.covers("crypto", Some("eth-usd")),
            "symbol match is case-insensitive"
        );
        assert!(
            !s.covers("equity", Some("SPY")),
            "a crypto scope labelling an equity"
        );
        assert!(
            !s.covers("crypto", Some("SOL-USD")),
            "an instrument nobody decided about"
        );
        assert!(
            !s.covers("crypto", None),
            "an explicit list cannot be checked without a symbol"
        );

        // An empty list is the wider claim, and it is only honest when the
        // operator wrote it down that way.
        let all = scope(&[]);
        assert!(all.covers("crypto", Some("SOL-USD")));
        assert!(all.covers("crypto", None));
        assert!(!all.covers("equity", Some("SPY")));
    }

    /// Two scopes for one asset class and no symbol is ambiguous, and ambiguity
    /// is reported rather than resolved by row order.
    #[test]
    fn two_markets_and_no_symbol_is_not_a_choice_to_make_quietly() {
        let one = scope(&["BTC-USD"]);
        let mut two = scope(&["ETH-USD"]);
        two.market_scope = "crypto_eth_usd".into();
        two.series_symbol = "ETH-USD".into();
        let both = vec![one.clone(), two.clone()];

        assert_eq!(
            resolve_from(&both, "crypto", Some("ETH-USD")),
            ScopeResolution::Scope(Box::new(two))
        );
        match resolve_from(&both, "crypto", None) {
            ScopeResolution::Ambiguous(names) => assert_eq!(names.len(), 2),
            other => panic!("two scopes, no symbol, and it picked one: {other:?}"),
        }
        // One scope for the class is unambiguous: there is only one market it
        // could mean.
        assert_eq!(
            resolve_from(&[one], "crypto", None),
            ScopeResolution::Scope(Box::new(scope(&["BTC-USD"])))
        );
        assert_eq!(
            resolve_from(&both, "equity", None),
            ScopeResolution::Unconfigured
        );
        assert_eq!(
            resolve_from(&both, "crypto", Some("SOL-USD")),
            ScopeResolution::Unconfigured
        );
    }

    /// The alignment is causal in both bounds. A label whose knowledge time is
    /// after the observation is the future, however old its event time looks.
    #[test]
    fn a_label_that_was_not_knowable_yet_is_not_the_answer() {
        let t = |d: u32| chrono::Utc.with_ymd_and_hms(2026, 1, d, 0, 0, 0).unwrap();
        let obs = |d: u32, k: u32, label: RegimeLabel| RegimeObservation {
            event_time: t(d),
            label,
            p_filtered: label.one_hot(),
            knowledge_time: t(k),
            series_log_return: Some(0.0),
        };
        let history = vec![
            obs(1, 1, RegimeLabel::LowVol),
            // Event time on the 2nd, but not knowable until the 9th: a
            // restatement, a late backfill, a labeller that ran a week later.
            obs(2, 9, RegimeLabel::HighVol),
            obs(3, 3, RegimeLabel::MidVol),
        ];
        let got = labels_as_of(&history, &[t(1), t(2), t(3), t(10)]);
        assert_eq!(
            got,
            vec![
                Some(RegimeLabel::LowVol),
                // NOT HighVol: on the 2nd the platform had not seen that row.
                Some(RegimeLabel::LowVol),
                Some(RegimeLabel::MidVol),
                Some(RegimeLabel::MidVol),
            ]
        );
        // Before any row exists there is no label, and no label is not a label.
        assert_eq!(
            labels_as_of(&history, &[t(1) - chrono::Duration::days(1)]),
            vec![None]
        );
    }

    /// There is no function in this module that reads the smoothed path. If one
    /// is ever added, this fails — which is the point: the grant would refuse it
    /// at runtime, and a compile-time absence is a better place to find out.
    #[test]
    fn this_module_offers_no_way_to_read_a_smoothed_regime() {
        // Everything above the test module, minus comments: the tests
        // themselves have to name what they forbid.
        let src = include_str!("knowledge.rs");
        let production = &src[..src.find("#[cfg(test)]").expect("a test module")];
        let code: String = production
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code.contains("regime_research") && !code.contains("p_smoothed"),
            "the knowledge module must not offer a smoothed read (AT-42, R-07)"
        );
    }
}

/// Contiguous runs of one causal label, as the diagnostics bundle's regime
/// attribution consumes them.
///
/// # Why this goes through `labels_as_of` rather than over the history directly
///
/// `backtest::stats::diagnostics::RegimeWindow` had **no producer anywhere in the
/// workspace**: `DiagnosticBundle::from_result` takes `&[RegimeWindow]` and its
/// only caller passed `&[]`, so every bundle this platform ever served reported
/// `by_regime` as `Unmeasured { NoLabelsSupplied }`. Honest, and never measured.
///
/// Building windows from `history` would mean deciding again when a label becomes
/// an answer, and there is already an answer to that: [`labels_as_of`] requires a
/// row's **event time and knowledge time both** to be at or before `t`, and
/// yields `None` rather than the nearest label, because nearest in that direction
/// is the future. Taking the labels from that function and grouping them keeps
/// one implementation of the rule. A second one is how a lookahead gets in.
///
/// So `times` should be the curve the attribution will run over — the run's own
/// equity-curve timestamps — and the windows come back aligned to those instants
/// by construction.
///
/// An unlabelled instant **ends** the window it follows rather than extending it.
/// A gap in the labelling is not the label either side of it.
///
/// `times` and `labels` must be the same length and `times` ascending;
/// [`labels_as_of`] returns labels in the order it was given times.
#[must_use]
pub fn windows_from_labels(
    times: &[DateTime<Utc>],
    labels: &[Option<RegimeLabel>],
) -> Vec<backtest::stats::diagnostics::RegimeWindow> {
    use backtest::stats::diagnostics::RegimeWindow;

    let mut out: Vec<RegimeWindow> = Vec::new();
    let mut run: Option<(RegimeLabel, DateTime<Utc>, DateTime<Utc>)> = None;

    for (t, l) in times.iter().zip(labels.iter()) {
        match (l, &mut run) {
            // Extending the current run.
            (Some(label), Some((open, _, last))) if *label == *open => *last = *t,
            // A different label, or the first labelled instant after a gap:
            // close what was open and start again.
            (Some(label), slot) => {
                if let Some((open, start, last)) = slot.take() {
                    out.push(window(open, start, last));
                }
                *slot = Some((*label, *t, *t));
            }
            // Unlabelled: nothing is in force, so the run ends here.
            (None, slot) => {
                if let Some((open, start, last)) = slot.take() {
                    out.push(window(open, start, last));
                }
            }
        }
    }
    if let Some((open, start, last)) = run {
        out.push(window(open, start, last));
    }
    out
}

/// One window over `[start, last]`, as a half-open `[start, end)`.
///
/// `end` is one nanosecond past the last labelled instant rather than the next
/// instant in the curve: `regime_attribution` selects equity points with
/// `ts >= start` and `ts < end`, so this includes `last` and cannot reach over an
/// instant the label was not in force for. Adjacent windows therefore leave a
/// sub-nanosecond gap that no equity point can fall into, which is the correct
/// trade — the alternative extends a label across a boundary to close a gap that
/// contains nothing.
fn window(
    label: RegimeLabel,
    start: DateTime<Utc>,
    last: DateTime<Utc>,
) -> backtest::stats::diagnostics::RegimeWindow {
    backtest::stats::diagnostics::RegimeWindow {
        start,
        end: last + chrono::Duration::nanoseconds(1),
        label: label.as_str().to_string(),
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("in range")
    }

    fn labels(v: &[Option<RegimeLabel>]) -> Vec<Option<RegimeLabel>> {
        v.to_vec()
    }

    #[test]
    fn contiguous_instants_of_one_label_are_one_window() {
        let times = [t(0), t(60), t(120)];
        let w = windows_from_labels(
            &times,
            &labels(&[
                Some(RegimeLabel::LowVol),
                Some(RegimeLabel::LowVol),
                Some(RegimeLabel::LowVol),
            ]),
        );
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].label, "low_vol");
        assert_eq!(w[0].start, t(0));
        // Half-open, and past the last labelled instant so that instant is inside
        // it: `regime_attribution` selects `ts < end`.
        assert!(w[0].end > t(120));
        assert!(w[0].end < t(121));
    }

    #[test]
    fn a_label_change_closes_one_window_and_opens_the_next() {
        let times = [t(0), t(60), t(120)];
        let w = windows_from_labels(
            &times,
            &labels(&[
                Some(RegimeLabel::LowVol),
                Some(RegimeLabel::HighVol),
                Some(RegimeLabel::HighVol),
            ]),
        );
        assert_eq!(w.len(), 2);
        assert_eq!((w[0].label.as_str(), w[0].start), ("low_vol", t(0)));
        assert_eq!((w[1].label.as_str(), w[1].start), ("high_vol", t(60)));
        // The first window cannot reach the instant the second owns.
        assert!(w[0].end <= w[1].start);
    }

    #[test]
    fn an_unlabelled_instant_ends_the_window_rather_than_extending_it() {
        // The rule `labels_as_of` states: an unlabelled observation is not the
        // label either side of it. A window that spanned the gap would attribute
        // returns to a regime nobody had established for them.
        let times = [t(0), t(60), t(120)];
        let w = windows_from_labels(
            &times,
            &labels(&[Some(RegimeLabel::MidVol), None, Some(RegimeLabel::MidVol)]),
        );
        assert_eq!(w.len(), 2, "one window either side of the gap, not one across");
        assert!(w[0].end <= t(60), "the first must not reach the unlabelled instant");
        assert_eq!(w[1].start, t(120));
    }

    #[test]
    fn a_gap_between_two_runs_of_the_same_label_is_still_two_windows() {
        // Same label either side, so a naive grouping on label alone would merge
        // them and silently claim the unlabelled middle.
        let times = [t(0), t(60), t(120), t(180)];
        let w = windows_from_labels(
            &times,
            &labels(&[
                Some(RegimeLabel::LowVol),
                None,
                None,
                Some(RegimeLabel::LowVol),
            ]),
        );
        assert_eq!(w.len(), 2);
        assert_eq!(w.iter().filter(|x| x.label == "low_vol").count(), 2);
    }

    #[test]
    fn no_labels_at_all_produce_no_windows_rather_than_one_empty_one() {
        // An empty vec is what `regime_attribution` reads as
        // `Unmeasured { NoLabelsSupplied }` — which is the truth here. A single
        // zero-width window would read as a measurement that found nothing.
        let times = [t(0), t(60)];
        assert!(windows_from_labels(&times, &labels(&[None, None])).is_empty());
        assert!(windows_from_labels(&[], &[]).is_empty());
    }

    #[test]
    fn one_labelled_instant_is_a_window_that_contains_it() {
        let w = windows_from_labels(&[t(500)], &labels(&[Some(RegimeLabel::HighVol)]));
        assert_eq!(w.len(), 1);
        assert!(w[0].start <= t(500) && w[0].end > t(500));
    }

    #[test]
    fn the_windows_come_from_labels_as_of_so_the_knowledge_clock_is_respected() {
        // End to end through the real rule: a row whose knowledge time is after
        // the instant asked about is not an answer for it. If `windows_from_labels`
        // ever stopped going through `labels_as_of`, this is what would break.
        let history = vec![RegimeObservation {
            event_time: t(0),
            label: RegimeLabel::HighVol,
            p_filtered: vec![0.1, 0.2, 0.7],
            // Knowable only at t(600) — six hundred seconds after the event.
            knowledge_time: t(600),
            series_log_return: None,
        }];
        let times = [t(60), t(300), t(900)];
        let resolved = labels_as_of(&history, &times);
        assert_eq!(
            resolved,
            vec![None, None, Some(RegimeLabel::HighVol)],
            "a label is not an answer before it was knowable"
        );

        let w = windows_from_labels(&times, &resolved);
        assert_eq!(w.len(), 1, "only the instant after the knowledge time");
        assert_eq!(w[0].start, t(900));
    }
}
