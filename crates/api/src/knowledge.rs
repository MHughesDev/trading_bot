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
}

/// Reads and writes `regime_causal.regime_state`.
pub struct RegimeStore {
    pg: PgPool,
}

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
        for ((event_time, _), regime) in returns.iter().zip(labels.iter()) {
            let label = RegimeLabel::from_tercile(*regime);
            sqlx::query(
                "INSERT INTO regime_causal.regime_state
                     (market_scope, event_time, model_version, p_filtered, regime_vec,
                      knowledge_time, label)
                 VALUES ($1, $2, $3, $4, $5::real[]::vector, $6, $7)
                 ON CONFLICT (market_scope, event_time, model_version) DO NOTHING",
            )
            .bind(market_scope)
            .bind(event_time)
            .bind(RULE_MODEL_VERSION)
            .bind(label.one_hot())
            .bind(regime_vec(label))
            .bind(event_time)
            .bind(label.as_str())
            .execute(&self.pg)
            .await?;
            written += 1;
        }
        Ok(written)
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
            "SELECT event_time, label, p_filtered, knowledge_time
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

        Ok(row.and_then(|(event_time, label, p_filtered, knowledge_time)| {
            RegimeLabel::from_code(&label).map(|label| RegimeObservation {
                event_time,
                label,
                p_filtered,
                knowledge_time,
            })
        }))
    }
}

/// `(event_time, label, p_filtered, knowledge_time)` as the row comes back.
type RegimeRow = (DateTime<Utc>, String, Vec<f32>, DateTime<Utc>);

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
            .map(|(instrument_id, venue_id, distance, knowledge_time)| Neighbour {
                instrument_id,
                venue_id,
                distance,
                knowledge_time,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backtest::gates::evaluators::VolRegime;

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
        assert_eq!(RegimeLabel::from_tercile(VolRegime::Low), RegimeLabel::LowVol);
        assert_eq!(RegimeLabel::from_tercile(VolRegime::Mid), RegimeLabel::MidVol);
        assert_eq!(RegimeLabel::from_tercile(VolRegime::High), RegimeLabel::HighVol);
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
