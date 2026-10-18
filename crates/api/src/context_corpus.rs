//! The attribution corpus, the weight fit and the promotion procedure
//! (AGENT-005 sections 19 and 22; Set S tasks S-9.10 and S-9.12; migration 0238).
//!
//! `harness::context::utility` attributes a run's rendered records and
//! `harness::context::fit` fits the selection weights and decides promotion. Both
//! were complete and neither had a store, so there was nowhere for a corpus to
//! accumulate and nothing that could run over one. This module is that:
//!
//! * [`record_run`] is the **intake**, called once when a run ends (the hook is
//!   named in the task note: the driver owns the `RunStore`). It writes the run's
//!   token sums and one row per rendered record. Append-only, and idempotent per
//!   run so a resumed run cannot be counted twice.
//! * [`refresh`] reads the corpus back: per policy version and per profile, the
//!   tier's `utility_rate` (a ratio of summed tokens, so independent of corpus
//!   order and reproducible from the rows) and, when the corpus is large enough,
//!   the fitted weights. It runs daily from [`spawn_context_corpus_job`] and says
//!   so plainly when there is nothing to read: an empty corpus is reported as
//!   empty, never as a utility of zero.
//! * [`run_promotion`] is the **procedure**: it reads both policies' corpora,
//!   builds the per-profile utility the promotion rule asks for, and records the
//!   outcome win or lose through `eval_store::record_promotion`. A candidate
//!   whose corpus is below `MIN_CORPUS` is refused and **recorded** as refused
//!   for that reason, rather than being promoted on a handful of records or
//!   silently skipped.
//!
//! What is deliberately not here: the scorecards. Pass rates come from the eval
//! suites run under each policy, which need live sessions (S-4.14, gate H-2), so
//! `run_promotion` takes them as arguments.

use std::collections::BTreeMap;

use harness::context::fit::{
    fit, promote, Fitted, Observation, PromotionRecord, TierUtility, MIN_CORPUS,
};
use harness::context::select::Signals;
use harness::context::store::{RecordId, RunStore, Utility};
use harness::context::utility::Attribution;
use harness::eval::scorecard::Scorecard;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum CorpusError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Store(#[from] crate::eval_store::EvalStoreError),
}

/// One rendered record as the corpus keeps it.
#[derive(Debug, Clone)]
pub struct RecordRow {
    pub kind: String,
    pub signals: Signals,
    pub utility: Utility,
}

/// What a run contributes to the corpus.
#[derive(Debug, Clone)]
pub struct RunEntry {
    pub run_id: Uuid,
    pub profile_id: String,
    pub tier: String,
    pub policy_version: String,
    pub rendered_tokens: u64,
    pub used_tokens: u64,
    pub indeterminate_share: f32,
    pub records: Vec<RecordRow>,
}

impl RunEntry {
    /// Build an entry from what a finished run holds: its store, the
    /// attribution over it and the signals each record was scored on.
    ///
    /// A record the attribution left `NotAttributed` is not kept: it was never
    /// classified, so it is not evidence either way.
    #[must_use]
    pub fn from_attribution(
        run_id: Uuid,
        profile_id: &str,
        tier: &str,
        policy_version: &str,
        store: &RunStore,
        attribution: &Attribution,
        signals: impl Fn(RecordId) -> Option<Signals>,
    ) -> Self {
        let records = attribution
            .per_record
            .iter()
            .filter(|(_, u)| **u != Utility::NotAttributed)
            .filter_map(|(id, u)| {
                let rec = store.get(*id)?;
                Some(RecordRow {
                    kind: rec.kind.as_str().to_string(),
                    signals: signals(*id)?,
                    utility: *u,
                })
            })
            .collect();
        Self {
            run_id,
            profile_id: profile_id.to_string(),
            tier: tier.to_string(),
            policy_version: policy_version.to_string(),
            rendered_tokens: attribution.rendered_tokens,
            used_tokens: attribution.used_tokens,
            indeterminate_share: attribution.indeterminate_share.clamp(0.0, 1.0),
            records,
        }
    }
}

fn signals_to_json(s: &Signals) -> serde_json::Value {
    serde_json::json!({
        "subject": s.subject, "id_overlap": s.id_overlap, "kind_prior": s.kind_prior,
        "recency": s.recency, "citations": s.citations, "same_tool": s.same_tool,
        "correction": s.correction, "expansion": s.expansion, "stickiness": s.stickiness,
    })
}

fn signals_from_json(v: &serde_json::Value) -> Signals {
    let f = |k: &str| v.get(k).and_then(serde_json::Value::as_f64).unwrap_or(0.0) as f32;
    Signals {
        subject: f("subject"),
        id_overlap: f("id_overlap"),
        kind_prior: f("kind_prior"),
        recency: f("recency"),
        citations: f("citations"),
        same_tool: f("same_tool"),
        correction: f("correction"),
        expansion: f("expansion"),
        stickiness: f("stickiness"),
    }
}

fn utility_name(u: Utility) -> &'static str {
    match u {
        Utility::Used => "used",
        Utility::Wanted => "wanted",
        Utility::Unused => "unused",
        _ => "indeterminate",
    }
}

fn utility_from(s: &str) -> Utility {
    match s {
        "used" => Utility::Used,
        "wanted" => Utility::Wanted,
        "unused" => Utility::Unused,
        _ => Utility::Indeterminate,
    }
}

/// Add one run to the corpus. Returns `false` when the run is already in it
/// (a resumed run must not count twice), `true` when rows were written.
///
/// Both inserts are one transaction: a run row without its records would put
/// denominators in the corpus with no records to fit.
///
/// # Errors
/// [`CorpusError::Sqlx`] if a write fails.
pub async fn record_run(pool: &PgPool, e: &RunEntry) -> Result<bool, CorpusError> {
    let mut tx = pool.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO context_attribution_run \
           (run_id, profile_id, tier, policy_version, rendered_tokens, used_tokens, indeterminate_share) \
         VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (run_id) DO NOTHING",
    )
    .bind(e.run_id)
    .bind(&e.profile_id)
    .bind(&e.tier)
    .bind(&e.policy_version)
    .bind(i64::try_from(e.rendered_tokens).unwrap_or(i64::MAX))
    .bind(i64::try_from(e.used_tokens.min(e.rendered_tokens)).unwrap_or(i64::MAX))
    .bind(e.indeterminate_share)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        tx.rollback().await?;
        return Ok(false);
    }
    for r in &e.records {
        sqlx::query(
            "INSERT INTO context_attribution_record (run_id, record_kind, signals, utility) \
             VALUES ($1,$2,$3,$4)",
        )
        .bind(e.run_id)
        .bind(&r.kind)
        .bind(signals_to_json(&r.signals))
        .bind(utility_name(r.utility))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// One policy's corpus, read back.
#[derive(Debug, Clone, Default)]
pub struct Corpus {
    pub policy_version: String,
    pub runs: usize,
    /// Per profile: `(used_tokens, rendered_tokens)` summed over its runs.
    pub per_profile: BTreeMap<String, (u64, u64)>,
    /// Per profile: the tier its runs were recorded under.
    pub tier_of: BTreeMap<String, String>,
    pub observations: Vec<Observation>,
}

impl Corpus {
    /// `used / rendered` for a profile, or `None` when nothing was rendered.
    /// Never `0.0` for "not measured".
    #[must_use]
    pub fn utility_rate(&self, profile_id: &str) -> Option<f64> {
        self.per_profile
            .get(profile_id)
            .filter(|(_, rendered)| *rendered > 0)
            .map(|(used, rendered)| *used as f64 / *rendered as f64)
    }
}

/// Read one policy version's corpus.
///
/// Ordered by `run_id` and record id so the observation order, and with it the
/// fingerprint recorded with a promotion, is a function of the rows alone.
///
/// # Errors
/// [`CorpusError::Sqlx`] if a read fails.
pub async fn load(pool: &PgPool, policy_version: &str) -> Result<Corpus, CorpusError> {
    let mut c = Corpus {
        policy_version: policy_version.to_string(),
        ..Corpus::default()
    };
    let runs = sqlx::query(
        "SELECT profile_id, tier, rendered_tokens, used_tokens FROM context_attribution_run \
          WHERE policy_version = $1 ORDER BY run_id",
    )
    .bind(policy_version)
    .fetch_all(pool)
    .await?;
    for r in &runs {
        let profile: String = r.get("profile_id");
        let rendered: i64 = r.get("rendered_tokens");
        let used: i64 = r.get("used_tokens");
        let e = c.per_profile.entry(profile.clone()).or_insert((0, 0));
        e.0 += u64::try_from(used).unwrap_or(0);
        e.1 += u64::try_from(rendered).unwrap_or(0);
        c.tier_of.insert(profile, r.get("tier"));
    }
    c.runs = runs.len();
    let rows = sqlx::query(
        "SELECT r.signals, r.utility FROM context_attribution_record r \
           JOIN context_attribution_run u ON u.run_id = r.run_id \
          WHERE u.policy_version = $1 ORDER BY r.run_id, r.id",
    )
    .bind(policy_version)
    .fetch_all(pool)
    .await?;
    c.observations = rows
        .iter()
        .map(|r| Observation {
            signals: signals_from_json(&r.get::<serde_json::Value, _>("signals")),
            utility: utility_from(&r.get::<String, _>("utility")),
        })
        .collect();
    Ok(c)
}

/// What a refresh found.
#[derive(Debug, Clone)]
pub struct PolicyReport {
    pub policy_version: String,
    pub runs: usize,
    pub observations: usize,
    /// `profile -> utility_rate`; a profile with nothing rendered is absent.
    pub utility_rate: BTreeMap<String, f64>,
    /// `None` when the corpus is below `MIN_CORPUS` on a side: the declared
    /// weights stand, and [`Self::note`] says why.
    pub fitted: Option<Fitted>,
    pub note: String,
}

/// Read every policy's corpus and report. Runs on whatever exists, including
/// nothing: an empty database yields an empty report, not an error and not a
/// zero.
///
/// # Errors
/// [`CorpusError::Sqlx`] if a read fails.
pub async fn refresh(pool: &PgPool) -> Result<Vec<PolicyReport>, CorpusError> {
    let versions: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT policy_version FROM context_attribution_run ORDER BY 1",
    )
    .fetch_all(pool)
    .await?;
    let mut out = Vec::new();
    for v in versions {
        let c = load(pool, &v).await?;
        let used = c
            .observations
            .iter()
            .filter(|o| matches!(o.utility, Utility::Used | Utility::Wanted))
            .count();
        let unused = c
            .observations
            .iter()
            .filter(|o| o.utility == Utility::Unused)
            .count();
        let (fitted, note) = if used >= MIN_CORPUS && unused >= MIN_CORPUS {
            (Some(fit(&c.observations)), "fitted".to_string())
        } else {
            (
                None,
                format!(
                    "corpus too small to fit: {used} used and {unused} unused records, \
                     {MIN_CORPUS} needed on each side; the declared weights stand"
                ),
            )
        };
        out.push(PolicyReport {
            policy_version: v,
            runs: c.runs,
            observations: c.observations.len(),
            utility_rate: c
                .per_profile
                .keys()
                .filter_map(|p| c.utility_rate(p).map(|r| (p.clone(), r)))
                .collect(),
            fitted,
            note,
        });
    }
    Ok(out)
}

/// Ask whether `candidate_version` may replace the baseline `v1`, and record the
/// answer (S-9.12).
///
/// The candidate's weights are the fit over **its own** corpus. When that corpus
/// is below `MIN_CORPUS` on either side the procedure does not call `promote`:
/// it records a refusal naming the sizes, so a `v2` that was proposed on too
/// little evidence is on the record as exactly that and a later, larger corpus
/// is a new attempt with a different fingerprint. The record is written either
/// way; the return value is what was written.
///
/// # Errors
/// [`CorpusError`] if a read or the record insert fails.
pub async fn run_promotion(
    pool: &PgPool,
    candidate_version: &str,
    baselines: &[Scorecard],
    candidates: &[Scorecard],
) -> Result<PromotionRecord, CorpusError> {
    let base = load(pool, "v1").await?;
    let cand = load(pool, candidate_version).await?;
    let fitted = fit(&cand.observations);
    let used = cand
        .observations
        .iter()
        .filter(|o| matches!(o.utility, Utility::Used | Utility::Wanted))
        .count();
    let unused = cand
        .observations
        .iter()
        .filter(|o| o.utility == Utility::Unused)
        .count();

    let record = if used < MIN_CORPUS || unused < MIN_CORPUS {
        PromotionRecord {
            policy_version: candidate_version.to_string(),
            weights: fitted.weights,
            corpus_size: cand.observations.len(),
            corpus_fingerprint: harness::context::fit::corpus_fingerprint(&cand.observations),
            promoted: false,
            refused: vec![format!(
                "the candidate's corpus is {used} used and {unused} unused records; \
                 {MIN_CORPUS} are needed on each side before a fit means anything"
            )],
            report: Vec::new(),
        }
    } else {
        let mut utility: BTreeMap<String, (TierUtility, TierUtility)> = BTreeMap::new();
        for card in candidates {
            let (Some(v1), Some(v2)) = (
                base.utility_rate(&card.profile_id),
                cand.utility_rate(&card.profile_id),
            ) else {
                continue; // promote() refuses a profile with no measurement
            };
            utility.insert(
                card.profile_id.clone(),
                (
                    TierUtility {
                        utility_rate: v1 as f32,
                        cost_tokens: base.per_profile[&card.profile_id].1,
                    },
                    TierUtility {
                        utility_rate: v2 as f32,
                        cost_tokens: cand.per_profile[&card.profile_id].1,
                    },
                ),
            );
        }
        promote(baselines, candidates, &utility).record(
            candidate_version,
            &fitted,
            &cand.observations,
        )
    };
    crate::eval_store::record_promotion(pool, &record, env!("CARGO_PKG_VERSION")).await?;
    Ok(record)
}

/// The daily corpus read. A minute and a half after boot; failures are logged,
/// never fatal. Logs the state of the corpus whether or not there is one, so
/// "the corpus is empty" is a line in the log and not an absence of one.
pub fn spawn_context_corpus_job(pool: PgPool) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(90)).await;
        loop {
            match refresh(&pool).await {
                Ok(reports) if reports.is_empty() => {
                    tracing::info!("context attribution corpus is empty: no run has recorded one yet");
                }
                Ok(reports) => {
                    for r in reports {
                        tracing::info!(
                            policy = %r.policy_version,
                            runs = r.runs,
                            records = r.observations,
                            profiles = r.utility_rate.len(),
                            note = %r.note,
                            "context attribution corpus"
                        );
                    }
                }
                Err(e) => tracing::warn!(error = %e, "context corpus refresh failed"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(24 * 3600)).await;
        }
    });
}
