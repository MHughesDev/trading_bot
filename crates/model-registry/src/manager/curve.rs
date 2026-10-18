//! The loss curve of a training run, kept (Set H H-5.5).
//!
//! The trainer publishes `{epoch, train_loss, val_loss}` on every progress
//! frame. [`ModelManager::apply_nats_progress`] merged each frame over the last
//! one in memory, so a run's snapshot only ever held its latest epoch. This
//! module appends each epoch to `mlops.training_metric` (migration 0201) and
//! reads the series back, which is what the Training Console's curves draw.
//!
//! A loss the trainer did not measure is `None`/NULL. The trainer sends NaN for a
//! run with no validation split, and a NaN that became `0` would draw a perfect
//! validation curve for a model that was never validated.

use serde_json::Value;
use uuid::Uuid;

use super::ModelManager;

/// One epoch of a run.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricPoint {
    pub epoch: i32,
    pub train_loss: Option<f64>,
    pub val_loss: Option<f64>,
}

/// The point a progress frame's metric object carries, or `None` for a frame
/// that is not an epoch report (phase changes, checkpoints, final metrics).
///
/// A non-finite or non-numeric loss is `None`, not a number.
pub fn point_from_metric(metric: &Value) -> Option<MetricPoint> {
    let epoch = metric.get("epoch")?.as_i64()?;
    let epoch = i32::try_from(epoch).ok().filter(|e| *e >= 0)?;
    let loss = |k: &str| {
        metric
            .get(k)
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
    };
    let (train_loss, val_loss) = (loss("train_loss"), loss("val_loss"));
    // An epoch frame that reports neither loss has nothing to draw.
    if train_loss.is_none() && val_loss.is_none() {
        return None;
    }
    Some(MetricPoint {
        epoch,
        train_loss,
        val_loss,
    })
}

impl ModelManager {
    /// Append the epoch a progress frame reports. Best-effort and idempotent: a
    /// resumed run re-reporting an epoch keeps the first row, and a store failure
    /// is logged rather than failing the frame, as with checkpoints.
    pub(super) async fn record_metric_point(&self, run_id: Uuid, metric: &Value) {
        let Some(p) = point_from_metric(metric) else {
            return;
        };
        if let Err(e) = sqlx::query(
            "INSERT INTO mlops.training_metric (run_id, epoch, train_loss, val_loss) \
             VALUES ($1,$2,$3,$4) ON CONFLICT (run_id, epoch) DO NOTHING",
        )
        .bind(run_id)
        .bind(p.epoch)
        .bind(p.train_loss)
        .bind(p.val_loss)
        .execute(&self.pg)
        .await
        {
            tracing::warn!(%run_id, error = %e, "epoch metric not stored");
        }
    }

    /// The recorded curve of a run, oldest epoch first. `Err` when the run is
    /// not the caller's model's: the curve of someone else's run is not served.
    pub async fn training_curve(
        &self,
        model_id: &str,
        run_id: Uuid,
        user_id: Uuid,
    ) -> anyhow::Result<Vec<MetricPoint>> {
        self.ensure_model_owned(model_id, user_id).await?;
        let owned: Option<(Uuid,)> =
            sqlx::query_as("SELECT run_id FROM training_runs WHERE run_id = $1 AND model_id = $2")
                .bind(run_id)
                .bind(model_id)
                .fetch_optional(&self.pg)
                .await?;
        anyhow::ensure!(owned.is_some(), "not found");
        let rows: Vec<(i32, Option<f64>, Option<f64>)> = sqlx::query_as(
            "SELECT epoch, train_loss, val_loss FROM mlops.training_metric \
             WHERE run_id = $1 ORDER BY epoch",
        )
        .bind(run_id)
        .fetch_all(&self.pg)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(epoch, train_loss, val_loss)| MetricPoint {
                epoch,
                train_loss,
                val_loss,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_epoch_frame_is_a_point() {
        let p =
            point_from_metric(&json!({"epoch": 3, "train_loss": 0.5, "val_loss": 0.7})).unwrap();
        assert_eq!(
            p,
            MetricPoint {
                epoch: 3,
                train_loss: Some(0.5),
                val_loss: Some(0.7)
            }
        );
    }

    #[test]
    fn an_unmeasured_loss_is_none_never_zero() {
        // serde_json parses `null`; NaN arrives as null or a string, never a number.
        let p =
            point_from_metric(&json!({"epoch": 1, "train_loss": 0.4, "val_loss": null})).unwrap();
        assert_eq!(p.val_loss, None);
        let p =
            point_from_metric(&json!({"epoch": 1, "train_loss": 0.4, "val_loss": "NaN"})).unwrap();
        assert_eq!(p.val_loss, None);
    }

    #[test]
    fn a_frame_that_is_not_an_epoch_report_is_not_a_point() {
        assert!(point_from_metric(&json!({"checkpoint": {"step": 2}})).is_none());
        assert!(point_from_metric(&json!({"epoch": -1, "train_loss": 0.1})).is_none());
        assert!(point_from_metric(&json!({"epoch": 2})).is_none());
        assert!(
            point_from_metric(&json!({"epoch": 2, "train_loss": null, "val_loss": null})).is_none()
        );
    }
}
