//! The statistical jump model's persistence and labelling pass (Set R-b RB-4.2, RB-4.4;
//! FEAT-003 §10).
//!
//! `research::regime_engine` is the pure model; this writes its fits to
//! `regime_causal.regime_jump_model` and its labels to `regime_causal.regime_state`,
//! the table the rule tier already fills, under a `model_version` of the form
//! `jump_v1:<fit date>`. The rule tier and the jump model therefore sit side by side in
//! one table and are compared by `model_version`, which is the A/B the ladder asks for.
//!
//! # Append-only, so a label never changes
//!
//! A label is inserted `ON CONFLICT DO NOTHING`. The filter is deterministic from the
//! fit's `train_start`, so relabelling the same window reproduces what is stored, and a
//! later pass only adds the bars that have arrived since. A refit is a new
//! `model_version`; it never rewrites an old one's labels.
//!
//! # The hand-off
//!
//! `apps/platform/src/regime_jobs.rs` owns the daily timer and reads the scope series.
//! After its `write_rule_tier` call it needs one more:
//! `api::knowledge_regime_jump::JumpLabeller::new(pg.clone()).run(&scope.market_scope, &returns).await`.
//! That file is outside this set's territory; the exact line is in the report.

use chrono::{DateTime, Utc};
use research::regime_engine::{self, features, JumpModel};
use serde_json::json;
use sqlx::{PgPool, Row};

use crate::knowledge::RegimeLabel;

/// Half-lives of the causal features, in observations (days for the daily scope series).
const HALF_LIFE_MEAN: f64 = 5.0;
const HALF_LIFE_VOL: f64 = 10.0;
/// The column of [`features`]' rows that holds volatility.
const VOL_COL: usize = 2;
/// A refit is due after this many days.
pub const REFIT_PERIOD_DAYS: i64 = 30;

/// What a pass did.
#[derive(Clone, Debug, PartialEq)]
pub struct JumpRun {
    pub model_version: String,
    pub refit: bool,
    pub labels_written: usize,
    pub median_run: f64,
}

pub struct JumpLabeller {
    pg: PgPool,
}

fn label_of(index: usize) -> RegimeLabel {
    match index {
        0 => RegimeLabel::LowVol,
        1 => RegimeLabel::MidVol,
        _ => RegimeLabel::HighVol,
    }
}

fn one_hot16(label: RegimeLabel) -> Vec<f32> {
    let mut v = vec![0.0_f32; 16];
    for (i, l) in RegimeLabel::ALL.iter().enumerate() {
        if *l == label {
            v[i] = 1.0;
        }
    }
    v
}

impl JumpLabeller {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    /// The scope's latest fit, if any: `(model_version, fitted_at, train_start, model)`.
    async fn latest(&self, scope: &str) -> Result<Option<(String, DateTime<Utc>, DateTime<Utc>, JumpModel)>, sqlx::Error> {
        let Some(r) = sqlx::query(
            "SELECT model_version, fitted_at, train_start, lambda, n_train, mean, sd, centroids, criterion
               FROM regime_causal.regime_jump_model WHERE market_scope = $1
              ORDER BY fitted_at DESC LIMIT 1",
        )
        .bind(scope)
        .fetch_optional(&self.pg)
        .await?
        else {
            return Ok(None);
        };
        let centroids: Vec<Vec<f64>> = serde_json::from_value(r.get("centroids")).unwrap_or_default();
        let criterion: Vec<(f64, f64)> = serde_json::from_value(r.get("criterion")).unwrap_or_default();
        Ok(Some((
            r.get("model_version"),
            r.get("fitted_at"),
            r.get("train_start"),
            JumpModel {
                lambda: r.get("lambda"),
                mean: r.get("mean"),
                sd: r.get("sd"),
                centroids,
                criterion,
                n_train: usize::try_from(r.get::<i32, _>("n_train")).unwrap_or(0),
            },
        )))
    }

    /// Label the scope's return series, refitting first when none exists or the last fit
    /// is older than [`REFIT_PERIOD_DAYS`]. `returns` is oldest first, `(bar close, log
    /// return)`, exactly what the rule tier is given.
    ///
    /// # Errors
    /// The database, or a series too short to fit.
    pub async fn run(&self, scope: &str, returns: &[(DateTime<Utc>, f64)]) -> Result<JumpRun, String> {
        let now = Utc::now();
        let existing = self.latest(scope).await.map_err(|e| e.to_string())?;
        let due = existing.as_ref().is_none_or(|(_, fitted, _, _)| {
            research::refit::refit_due(Some((now - *fitted).num_days()), REFIT_PERIOD_DAYS)
        });
        let r: Vec<f64> = returns.iter().map(|(_, x)| *x).collect();
        let rows = features(&r, HALF_LIFE_MEAN, HALF_LIFE_VOL);

        let (version, train_start, model, refit) = if due {
            let model = regime_engine::fit(&rows, VOL_COL).map_err(|e| e.to_string())?;
            let train_start = returns.first().map(|(t, _)| *t).ok_or("empty series")?;
            let version = format!("jump_v1:{}", now.date_naive());
            sqlx::query(
                "INSERT INTO regime_causal.regime_jump_model
                   (model_version, market_scope, lambda, n_train, train_start, mean, sd, centroids, criterion)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT (model_version) DO NOTHING",
            )
            .bind(&version)
            .bind(scope)
            .bind(model.lambda)
            .bind(i32::try_from(model.n_train).unwrap_or(i32::MAX))
            .bind(train_start)
            .bind(&model.mean)
            .bind(&model.sd)
            .bind(json!(model.centroids))
            .bind(json!(model.criterion))
            .execute(&self.pg)
            .await
            .map_err(|e| e.to_string())?;
            (version, train_start, model, true)
        } else {
            let (v, _, start, m) = existing.ok_or("no model")?;
            (v, start, m, false)
        };

        // The filter's state depends on where it starts, so start where the fit did.
        let from = returns.iter().position(|(t, _)| *t >= train_start).unwrap_or(0);
        let labels = model.label(&rows[from..]);
        let mut written = 0usize;
        for ((event_time, series_return), idx) in returns[from..].iter().zip(&labels) {
            let label = label_of(*idx);
            let done = sqlx::query(
                "INSERT INTO regime_causal.regime_state
                     (market_scope, event_time, model_version, p_filtered, regime_vec,
                      knowledge_time, label, series_log_return)
                 VALUES ($1, $2, $3, $4, $5::real[]::vector, $6, $7, $8)
                 ON CONFLICT (market_scope, event_time, model_version) DO NOTHING",
            )
            .bind(scope)
            .bind(event_time)
            .bind(&version)
            .bind(label.one_hot())
            .bind(one_hot16(label))
            // The bar's own close: the earliest instant its label could have been known.
            .bind(event_time)
            .bind(label.as_str())
            .bind(series_return)
            .execute(&self.pg)
            .await
            .map_err(|e| e.to_string())?;
            written += usize::try_from(done.rows_affected()).unwrap_or(0);
        }
        Ok(JumpRun {
            model_version: version,
            refit,
            labels_written: written,
            median_run: regime_engine::median_run_length(&labels),
        })
    }
}
