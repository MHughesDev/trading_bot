//! The evaluation side's persisted state: token calibration and holdout draws
//! (AGENT-009 sections 4.2 and 5; Set S tasks S-4.12 and S-4.17).
//!
//! Both are things the eval harness remembers about itself, and both exist
//! because a number that is not recorded cannot be audited:
//!
//! * [`refresh_calibration`] reads `agent_runs` (`prompt_tokens` from the provider
//!   against `est_prompt_tokens` from the harness, 0062) and writes one
//!   `agent_token_calibration` row per model with at least ten runs. A model the
//!   job has not seen gets no row, and [`bytes_per_token`] answers the prior.
//! * [`record_holdout_draw`] records a draw of the sealed eval set with the
//!   harness version that asked for it, and refuses a re-draw before the corpus
//!   has grown by 50% (`Holdout::refuse_redraw`).

use harness::context::{calibrate_bytes_per_token, TokenCalibration, PRIOR_BYTES_PER_TOKEN};
use harness::eval::Holdout;
use sqlx::{PgPool, Row};

#[derive(Debug, thiserror::Error)]
pub enum EvalStoreError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    /// A re-draw the holdout discipline refuses, with its reason.
    #[error("{0}")]
    RedrawRefused(String),
}

/// Recompute every model's calibration from its last 50 runs.
///
/// Returns the models written. A model with fewer than ten usable runs is left
/// alone (no row, or its previous row), because a percentile of nine numbers is a
/// number about those nine.
///
/// # Errors
/// [`EvalStoreError::Sqlx`] if a read or the upsert fails.
pub async fn refresh_calibration(
    pool: &PgPool,
) -> Result<Vec<(String, TokenCalibration)>, EvalStoreError> {
    let rows = sqlx::query(
        "SELECT model, est_prompt_tokens, prompt_tokens FROM ( \
           SELECT model, est_prompt_tokens, prompt_tokens, \
                  row_number() OVER (PARTITION BY model ORDER BY created_at DESC) AS rn \
             FROM agent_runs \
            WHERE prompt_tokens IS NOT NULL AND est_prompt_tokens IS NOT NULL \
              AND prompt_tokens > 0 AND est_prompt_tokens > 0 \
              AND model IS NOT NULL AND btrim(model) <> '' \
         ) t WHERE rn <= 50 ORDER BY model, rn",
    )
    .fetch_all(pool)
    .await?;

    let mut by_model: std::collections::BTreeMap<String, Vec<(u64, u64)>> =
        std::collections::BTreeMap::new();
    for r in &rows {
        let model: String = r.get("model");
        let est: i64 = r.get("est_prompt_tokens");
        let act: i64 = r.get("prompt_tokens");
        by_model
            .entry(model)
            .or_default()
            .push((u64::try_from(est).unwrap_or(0), u64::try_from(act).unwrap_or(0)));
    }

    let mut written = Vec::new();
    for (model, pairs) in by_model {
        let Some(c) = calibrate_bytes_per_token(&pairs) else {
            continue;
        };
        sqlx::query(
            "INSERT INTO agent_token_calibration \
               (model, bytes_per_token, sample_n, over_estimate_rate) \
             VALUES ($1,$2,$3,$4) \
             ON CONFLICT (model) DO UPDATE SET \
               bytes_per_token = EXCLUDED.bytes_per_token, \
               sample_n = EXCLUDED.sample_n, \
               over_estimate_rate = EXCLUDED.over_estimate_rate, \
               updated_at = now()",
        )
        .bind(&model)
        .bind(c.bytes_per_token)
        .bind(i32::try_from(c.sample_n).unwrap_or(i32::MAX))
        .bind(c.over_estimate_rate)
        .execute(pool)
        .await?;
        written.push((model, c));
    }
    Ok(written)
}

/// The chars-per-token to use for `model`: its calibrated row, or the 4.0 prior
/// for a model with no row.
///
/// # Errors
/// [`EvalStoreError::Sqlx`] if the read fails.
pub async fn bytes_per_token(pool: &PgPool, model: &str) -> Result<f64, EvalStoreError> {
    let v: Option<f64> =
        sqlx::query_scalar("SELECT bytes_per_token FROM agent_token_calibration WHERE model = $1")
            .bind(model)
            .fetch_optional(pool)
            .await?;
    Ok(v.unwrap_or(PRIOR_BYTES_PER_TOKEN))
}

/// The lowest over-estimate rate across calibrated models, or `None` when no
/// model has a row. What the budget suite reports (acceptance: stays >= 0.95).
///
/// # Errors
/// [`EvalStoreError::Sqlx`] if the read fails.
pub async fn worst_over_estimate_rate(pool: &PgPool) -> Result<Option<f64>, EvalStoreError> {
    Ok(
        sqlx::query_scalar("SELECT min(over_estimate_rate) FROM agent_token_calibration")
            .fetch_one(pool)
            .await?,
    )
}

/// Record a draw of the sealed eval set, with the version that asked for it.
///
/// A first draw is always accepted. A later one is refused unless the corpus has
/// grown by 50% since the most recent draw, so the holdout cannot be re-drawn on
/// demand to hide a regression.
///
/// # Errors
/// [`EvalStoreError::RedrawRefused`] with the reason; [`EvalStoreError::Sqlx`]
/// otherwise (including a repeated salt, which the table refuses).
pub async fn record_holdout_draw(
    pool: &PgPool,
    holdout: &Holdout,
    harness_version: &str,
) -> Result<(), EvalStoreError> {
    let last: Option<i32> = sqlx::query_scalar(
        "SELECT corpus_size FROM eval_holdout_draw ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    if let Some(prev) = last {
        let prev_draw = Holdout {
            draw: harness::eval::HoldoutDraw {
                corpus_size: usize::try_from(prev).unwrap_or(0),
                ..holdout.draw.clone()
            },
            sealed: Default::default(),
        };
        if let Some(why) = prev_draw.refuse_redraw(holdout.draw.corpus_size) {
            return Err(EvalStoreError::RedrawRefused(why));
        }
    }
    let drawn_on = chrono::NaiveDate::parse_from_str(&holdout.draw.drawn_on, "%Y-%m-%d")
        .unwrap_or_else(|_| chrono::Utc::now().date_naive());
    let sealed: Vec<String> = holdout.sealed.iter().cloned().collect();
    sqlx::query(
        "INSERT INTO eval_holdout_draw \
           (salt, requested_by, harness_version, drawn_on, corpus_size, sealed_ids) \
         VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(&holdout.draw.salt)
    .bind(&holdout.draw.requested_by)
    .bind(harness_version)
    .bind(drawn_on)
    .bind(i32::try_from(holdout.draw.corpus_size).unwrap_or(i32::MAX))
    .bind(&sealed)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record a promotion attempt, win or lose, with its weights and corpus
/// fingerprint (S-9.12).
///
/// # Errors
/// [`EvalStoreError::Sqlx`] if the insert fails.
pub async fn record_promotion(
    pool: &PgPool,
    rec: &harness::context::fit::PromotionRecord,
    harness_version: &str,
) -> Result<(), EvalStoreError> {
    sqlx::query(
        "INSERT INTO context_policy_promotion            (policy_version, weights_json, corpus_size, corpus_fingerprint, promoted,             refused, report, harness_version)          VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(&rec.policy_version)
    .bind(serde_json::to_value(rec.weights).unwrap_or(serde_json::Value::Null))
    .bind(i32::try_from(rec.corpus_size).unwrap_or(i32::MAX))
    .bind(&rec.corpus_fingerprint)
    .bind(rec.promoted)
    .bind(&rec.refused)
    .bind(&rec.report)
    .bind(harness_version)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record the standing sealed-set draw, once (S-4.17).
///
/// `cases::standard_holdout` is the draw CI iterates; until this ran, nothing
/// recorded it, so "who drew it, when, against how large a corpus" had no
/// answer and a re-draw had nothing to be refused against. Returns `true` when
/// a row was written, `false` when this salt is already on record or the
/// holdout discipline refuses a second draw (the refusal is the point: a changed
/// salt over a corpus that has not grown by 50% stays unrecorded and logged).
///
/// # Errors
/// [`EvalStoreError::Sqlx`] if a read or the insert fails.
pub async fn record_standing_draw(pool: &PgPool) -> Result<bool, EvalStoreError> {
    let h = harness::eval::cases::standard_holdout();
    let already: Option<i32> =
        sqlx::query_scalar("SELECT 1 FROM eval_holdout_draw WHERE salt = $1")
            .bind(&h.draw.salt)
            .fetch_optional(pool)
            .await?;
    if already.is_some() {
        return Ok(false);
    }
    match record_holdout_draw(pool, &h, env!("CARGO_PKG_VERSION")).await {
        Ok(()) => Ok(true),
        Err(EvalStoreError::RedrawRefused(why)) => {
            tracing::warn!(%why, salt = %h.draw.salt, "the standing eval holdout re-draw was refused");
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

/// The eval side's two standing callers: record the standing holdout draw once,
/// then refresh every model's token calibration daily (S-4.12, S-4.17).
///
/// A minute after boot so migrations have settled; failures are logged, never
/// fatal -- the loop reads the prior when no calibration row exists. Needs one
/// line in the platform's `main.rs`: `api::eval_store::spawn_eval_jobs(pg.clone());`.
#[allow(dead_code)] // the lib calls it; a test that compiles this file in via `#[path]` does not
pub fn spawn_eval_jobs(pool: PgPool) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        match record_standing_draw(&pool).await {
            Ok(true) => tracing::info!("standing eval holdout draw recorded"),
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, "recording the standing eval holdout draw failed"),
        }
        loop {
            match refresh_calibration(&pool).await {
                Ok(rows) => {
                    for (model, c) in rows {
                        tracing::info!(model, bytes_per_token = c.bytes_per_token, "token calibration refreshed");
                    }
                }
                Err(e) => tracing::warn!(error = %e, "token calibration refresh failed"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(24 * 3600)).await;
        }
    });
}
