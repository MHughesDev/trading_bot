//! Calling a trained model with the features it was trained on.
//!
//! # Why this module exists
//!
//! `model_registry::inference_gateway` is complete and careful: it resolves an
//! alias to a version, caches, breaks the circuit after repeated failures,
//! dispatches to the sidecar, falls back, and **abstains** rather than inventing
//! a number. Until this module, it had exactly one caller in the workspace, and
//! that caller did this:
//!
//! ```ignore
//! let features_map = HashMap::<String, f64>::new();      // empty
//! state.inference.forecast(&id, &alias, &id, &features_map).await
//! //                             model_id ^^^   ^^^ passed again as instrument_id
//! ```
//!
//! So the platform asked its models to forecast from **no features at all**, and
//! keyed the trace on a model id in the instrument slot. `refresh_node_inferences`
//! — the API designed to take real features — had no callers whatsoever. No
//! trained model on this platform has ever been called with real inputs, which is
//! also why `model_predictions` is empty.
//!
//! # The rule this module is built around
//!
//! **A model called with a feature map that does not match its training columns
//! returns a confident wrong answer.** There is no error, because every number it
//! received was a valid number. This repository has already shipped that bug once
//! (the sorted-feature serving defect) and the fix was to make the trainer and
//! the server agree on an *ordered* column list.
//!
//! So the order here is: read the columns the version declares, compute exactly
//! those, and **refuse by name** if any cannot be produced. Never pad with zero,
//! never drop a column, never reorder. A refusal naming three missing features is
//! worth more than a prediction that silently treated them as zero — the refusal
//! can be fixed, and the prediction cannot even be detected.
//!
//! That is also why this does not reuse `backtest::warmup::run_indicators`, which
//! is otherwise exactly the right function: it ends in
//! `filter_map(|(k, v)| Some((k, v?)))`, so a feature with no value at the last
//! bar is **silently dropped from the map**. For a backtest warmup that is
//! harmless. For a model call it is the defect above, arriving quietly.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

use backtest::store::BarStore;
use domain::model::forecast::CalibratedForecast;
use backtest::types::TimeframeExt;
use backtest::warmup::feature_row;
use domain::payloads::bar::Timeframe;

/// What a caller asks for.
#[derive(Debug, Clone)]
pub struct CallRequest {
    /// A model slug or a model id; the gateway resolves either.
    pub model_ref: String,
    /// Which published version to call. Defaults to `production`.
    pub alias: String,
    /// The instrument to predict for. **Not** the model id — the gateway keys its
    /// cache and its trace on this, so passing anything else corrupts both.
    pub instrument_id: String,
    pub timeframe: Timeframe,
    /// The point in time to predict at. Clamped to now: a caller cannot ask what
    /// the model will say tomorrow.
    pub as_of: DateTime<Utc>,
}

/// One feature the model wanted and this call could not supply.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "why", rename_all = "snake_case")]
pub enum MissingFeature {
    /// The feature engine has no implementation of this name. A model trained
    /// against a column this build cannot compute is a version this build cannot
    /// serve, and saying which name is the whole of the fix.
    UnknownToTheEngine { name: String },
    /// The engine knows the feature and produced no value at `as_of` — almost
    /// always too little history before it for the lookback the name declares.
    /// Distinct from the above because the remedy is different: load more bars,
    /// not build a feature.
    NoValueAtAsOf { name: String },
}

/// The outcome of asking a trained model for a prediction.
///
/// Every variant that is not [`Self::Predicted`] names what stopped it. There is
/// deliberately no variant meaning "no prediction, and here is a neutral number
/// anyway": a caller that wants to treat abstention as flat has to write that
/// down itself.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ModelCall {
    Predicted {
        model_id: String,
        version: i32,
        alias: String,
        instrument_id: String,
        timeframe: String,
        as_of: DateTime<Utc>,
        direction: String,
        confidence: f64,
        /// The published I-6.1 predict contract, present when the model returns
        /// a full distribution rather than a point direction.
        ///
        /// Built with the **real** `instrument_id`. The route this replaced built
        /// it with the model id in that field, so every calibrated forecast it
        /// ever published was labelled with the wrong instrument —
        /// `serve_path_quantiles_match_eval_path` pins the quantiles and could
        /// not catch that, because the quantiles were right and the subject was
        /// wrong.
        #[serde(skip_serializing_if = "Option::is_none")]
        calibrated: Option<serde_json::Value>,
        /// The ordered column list actually sent, so a reader can check it
        /// against the version's declared set rather than trusting this.
        features_sent: Vec<String>,
    },
    /// The gateway declined. This is the model's own answer and is not a failure
    /// of this call — a circuit that has broken open, a sidecar that is not
    /// running, or an artifact that cannot be fetched all arrive here.
    Abstained {
        model_id: String,
        version: i32,
        /// What the gateway could tell us. It returns `Option`, not a reason, so
        /// this names the observable rather than claiming to know which.
        detail: String,
    },
    NoSuchModel {
        model_ref: String,
    },
    NoSuchAlias {
        model_id: String,
        alias: String,
    },
    /// The version declares no feature set, so there is no column list to
    /// reproduce. Refused rather than guessed: a model whose training columns
    /// were not recorded cannot be served correctly by anyone.
    NoFeatureSet {
        model_id: String,
        version: i32,
    },
    /// At least one declared column could not be produced. Carries the full
    /// declared list beside the failures, because "three missing" means something
    /// different out of four columns than out of forty.
    FeaturesUnavailable {
        model_id: String,
        version: i32,
        required: Vec<String>,
        missing: Vec<MissingFeature>,
    },
    /// No bars at all for this instrument and timeframe at or before `as_of`.
    /// Separate from a feature with no value: nothing was read, so nothing about
    /// the features has been established.
    NoBars {
        instrument_id: String,
        timeframe: String,
        as_of: DateTime<Utc>,
    },
}

/// How many bars to read behind `as_of`.
///
/// The longest lookback any registered feature name can declare is bounded by the
/// period in the name, and nothing in the catalogue reads beyond a few hundred
/// bars. This is generous rather than computed because reading more bars than a
/// feature needs is cheap and getting it wrong in the other direction produces
/// [`MissingFeature::NoValueAtAsOf`] on a feature that is perfectly computable —
/// a refusal the operator cannot act on, which is the worst kind.
const LOOKBACK_BARS: u64 = 1_000;

/// Call a trained model for one instrument at one instant.
///
/// # Errors
/// Never returns `Err` for a condition the caller should see: every refusal is a
/// [`ModelCall`] variant naming itself. `Err` is reserved for the database being
/// unreachable, which is not a fact about the model.
pub async fn call_model(
    pg: &PgPool,
    inference: &model_registry::inference_gateway::InferenceGateway,
    clickhouse_url: &str,
    req: &CallRequest,
) -> Result<ModelCall, sqlx::Error> {
    let as_of = req.as_of.min(Utc::now());

    // ── 1. The model, and which version this alias points at ─────────────────
    let model_id: Option<(String,)> =
        sqlx::query_as("SELECT model_id FROM ai_models WHERE slug = $1 OR model_id = $1 LIMIT 1")
            .bind(&req.model_ref)
            .fetch_optional(pg)
            .await?;
    let Some((model_id,)) = model_id else {
        return Ok(ModelCall::NoSuchModel {
            model_ref: req.model_ref.clone(),
        });
    };

    let version: Option<(i32,)> = sqlx::query_as(
        "SELECT version FROM ai_model_aliases WHERE model_id = $1 AND alias = $2",
    )
    .bind(&model_id)
    .bind(&req.alias)
    .fetch_optional(pg)
    .await?;
    let Some((version,)) = version else {
        return Ok(ModelCall::NoSuchAlias {
            model_id,
            alias: req.alias.clone(),
        });
    };

    // ── 2. The columns that version was trained on, in order ─────────────────
    let required = declared_features(pg, &model_id, version).await?;
    let Some(required) = required else {
        return Ok(ModelCall::NoFeatureSet { model_id, version });
    };

    // ── 3. Bars at or before `as_of`, read point-in-time ─────────────────────
    // `BarStore::as_of` is the horizon for the whole read, so a bar the platform
    // could not have known at `as_of` is not visible here even if it is stored
    // now. That is the same read the backtest uses, which is what makes a
    // prediction comparable to one made during a replay.
    let store = BarStore::connect(clickhouse_url).as_of(as_of);
    // Saturating rather than panicking: a timeframe whose lookback overflows an
    // `i64` of seconds cannot exist, but a `expect` here would be a crash in a
    // read path over an arithmetic claim nobody checks.
    let lookback_secs =
        i64::try_from(req.timeframe.seconds().saturating_mul(LOOKBACK_BARS)).unwrap_or(i64::MAX);
    let from = as_of - chrono::Duration::seconds(lookback_secs);
    let bars = store
        .load_bars(&req.instrument_id, req.timeframe, from, as_of)
        .await
        .unwrap_or_default();
    if bars.is_empty() {
        return Ok(ModelCall::NoBars {
            instrument_id: req.instrument_id.clone(),
            timeframe: req.timeframe.key().to_string(),
            as_of,
        });
    }

    // ── 4. Exactly those columns, or a refusal naming what is missing ────────
    let rows: Vec<features::FeatureRow> = bars.iter().map(feature_row).collect();
    let sent = match assemble(&required, &rows) {
        Ok(sent) => sent,
        Err(missing) => {
            return Ok(ModelCall::FeaturesUnavailable {
                model_id,
                version,
                required,
                missing,
            });
        }
    };

    // ── 5. Ask the model ────────────────────────────────────────────────────
    let model_id_for_contract = model_id.clone();
    match inference
        .forecast(&req.model_ref, &req.alias, &req.instrument_id, &sent)
        .await
    {
        Some(f) => Ok(ModelCall::Predicted {
            model_id,
            version,
            alias: req.alias.clone(),
            instrument_id: req.instrument_id.clone(),
            timeframe: req.timeframe.key().to_string(),
            as_of,
            direction: f.direction.clone(),
            confidence: f.confidence,
            // A distribution that fails `validate()` is dropped rather than
            // published: `from_distribution` is the only thing that checks the
            // quantiles are monotone, and a forecast that failed that check is
            // not a forecast. The point direction and confidence above still
            // stand — they do not depend on the quantile ladder.
            calibrated: f.distribution.as_ref().and_then(|d| {
                CalibratedForecast::from_distribution(
                    model_id_for_contract.clone(),
                    u32::try_from(version).unwrap_or(0),
                    req.instrument_id.clone(),
                    req.timeframe.key().to_string(),
                    as_of,
                    d.clone(),
                )
                .ok()
                .and_then(|cf| serde_json::to_value(cf).ok())
            }),
            features_sent: required,
        }),
        None => Ok(ModelCall::Abstained {
            model_id,
            version,
            detail: "the inference gateway returned no forecast: the circuit may be open \
                     after repeated failures, the sidecar may not be running, or the \
                     artifact may be unfetchable. It declined to answer rather than \
                     guessing, and so does this."
                .to_string(),
        }),
    }
}

/// Exactly the declared columns, or every reason one is missing.
///
/// Pure: rows in, map or refusals out. No database, no gateway, no clock — which
/// is what lets the rule this whole module exists for be tested directly rather
/// than inferred from an integration run that needs a trained model to exist.
///
/// Two properties are load-bearing and both are asserted below:
///
/// 1. **A missing column is never substituted.** Not with zero, not with a mean,
///    not by omission. The returned map either has every declared name or the
///    call does not happen.
/// 2. **Every missing column is named**, not just the first. Resolved one at a
///    time rather than through `features::runtime::latest`, which collects into a
///    `Result` and therefore stops at the first unknown name — an operator fixing
///    a feature set wants the whole list, not the alphabetically unluckiest entry.
///
/// # Errors
/// The full set of [`MissingFeature`]s, in the declared order.
pub fn assemble(
    required: &[String],
    rows: &[features::FeatureRow],
) -> Result<HashMap<String, f64>, Vec<MissingFeature>> {
    let Some(last) = rows.len().checked_sub(1) else {
        // No rows is not "every feature is missing": nothing was read, so nothing
        // about the features has been established. The caller reports `NoBars`
        // before reaching here, and this guard keeps that true if it ever stops.
        return Err(required
            .iter()
            .map(|name| MissingFeature::NoValueAtAsOf { name: name.clone() })
            .collect());
    };

    let mut sent: HashMap<String, f64> = HashMap::with_capacity(required.len());
    let mut missing: Vec<MissingFeature> = Vec::new();
    for name in required {
        match features::runtime::feature(name) {
            Err(_) => missing.push(MissingFeature::UnknownToTheEngine { name: name.clone() }),
            Ok(f) => match features::runtime::value_at(f.as_ref(), rows, last) {
                Some(v) => {
                    sent.insert(name.clone(), v);
                }
                None => missing.push(MissingFeature::NoValueAtAsOf { name: name.clone() }),
            },
        }
    }
    if missing.is_empty() {
        Ok(sent)
    } else {
        Err(missing)
    }
}

/// The ordered column list a version was trained on, or `None` when it recorded
/// no feature set.
async fn declared_features(
    pg: &PgPool,
    model_id: &str,
    version: i32,
) -> Result<Option<Vec<String>>, sqlx::Error> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT feature_set_ref FROM ai_model_versions WHERE model_id = $1 AND version = $2",
    )
    .bind(model_id)
    .bind(version)
    .fetch_optional(pg)
    .await?;

    let Some((Some(feature_set_ref),)) = row else {
        return Ok(None);
    };

    let list: Option<(serde_json::Value,)> = sqlx::query_as(
        "SELECT feature_list_json FROM feature_sets WHERE feature_set_id = $1 OR name = $1",
    )
    .bind(&feature_set_ref)
    .fetch_optional(pg)
    .await?;

    // A feature set that exists with an empty or unreadable list is the same
    // problem as no feature set at all, and is reported as such rather than as a
    // successful read of nothing.
    Ok(list
        .and_then(|(v,)| serde_json::from_value::<Vec<String>>(v).ok())
        .filter(|l| !l.is_empty()))
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic bars, one hour apart, drifting up with a zig-zag on top.
    ///
    /// The oscillation is not decoration: a monotonically rising close gives RSI
    /// no losses at all, so the family divides by a zero average loss and reports
    /// no value — a fixture that would have made this test assert the wrong thing
    /// about the code.
    fn rows(n: usize) -> Vec<features::FeatureRow> {
        (0..n)
            .map(|i| {
                let drift = 100.0 + i as f64 * 0.5;
                let close = if i % 2 == 0 { drift } else { drift - 0.9 };
                features::FeatureRow {
                    ts_ns: i as i64 * 3_600_000_000_000,
                    open: close - 0.2,
                    high: close + 0.4,
                    low: close - 0.4,
                    close,
                    volume: 1_000.0,
                }
            })
            .collect()
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_complete_set_sends_exactly_the_declared_columns_and_no_others() {
        let required = names(&["close", "ema_5", "rsi_14"]);
        // 120 bars because `rsi_14` declares a lookback of
        // `14 * RSI_LOOKBACK_PERIODS + 1` = 71, and a fixture sized to the period
        // rather than to the declared lookback is how this test first claimed the
        // code was broken when it was not.
        let sent = assemble(&required, &rows(120)).unwrap_or_else(|m| {
            panic!("expected all three computable over 120 bars, missing: {m:?}")
        });

        // Exactly the declared set: a model that was trained on three columns and
        // receives four is as wrong as one that receives two.
        assert_eq!(sent.len(), 3, "sent {sent:?}");
        for n in &required {
            assert!(sent.contains_key(n), "{n} missing from {sent:?}");
        }
    }

    #[test]
    fn a_column_the_engine_cannot_compute_is_named_rather_than_dropped() {
        // The defect this guards: `run_indicators` ends in
        // `filter_map(|(k, v)| Some((k, v?)))`, so an uncomputable column leaves
        // the map silently and the model is called with a short vector. Here it
        // must be a refusal that says the name out loud.
        let required = names(&["close", "frobnicate_12"]);
        let missing =
            assemble(&required, &rows(120)).expect_err("frobnicate_12 is not a feature");

        assert_eq!(
            missing,
            vec![MissingFeature::UnknownToTheEngine {
                name: "frobnicate_12".to_string()
            }]
        );
    }

    #[test]
    fn too_little_history_is_a_different_refusal_from_an_unknown_name() {
        // `ema_200` exists and is perfectly computable — over 200 bars. Over ten
        // it has no value, and the remedy is to load more bars rather than to
        // build a feature. Collapsing the two would send an operator to fix the
        // wrong thing.
        let missing = assemble(&names(&["ema_200"]), &rows(10))
            .expect_err("200-bar lookback over 10 bars has no value");

        assert_eq!(
            missing,
            vec![MissingFeature::NoValueAtAsOf {
                name: "ema_200".to_string()
            }]
        );
    }

    #[test]
    fn every_missing_column_is_named_not_only_the_first() {
        // `features::runtime::latest` collects into a `Result` and stops at the
        // first unknown name. An operator repairing a feature set needs the whole
        // list, so this resolves each name independently.
        let required = names(&["close", "frobnicate_12", "ema_200", "not_a_feature_either"]);
        let missing = assemble(&required, &rows(10)).expect_err("three of four are unavailable");

        assert_eq!(missing.len(), 3, "got {missing:?}");
        assert!(missing.contains(&MissingFeature::UnknownToTheEngine {
            name: "frobnicate_12".to_string()
        }));
        assert!(missing.contains(&MissingFeature::UnknownToTheEngine {
            name: "not_a_feature_either".to_string()
        }));
        assert!(missing.contains(&MissingFeature::NoValueAtAsOf {
            name: "ema_200".to_string()
        }));
    }

    #[test]
    fn a_refused_assembly_hands_back_no_partial_map_to_send_anyway() {
        // The whole rule, stated as a type: on refusal there is no map at all, so
        // a caller cannot reach for "what we did manage to compute" and send that.
        // A caller that wants to substitute a zero has to write the zero itself.
        let out = assemble(&names(&["close", "frobnicate_12"]), &rows(120));
        assert!(out.is_err());
        // And `close` — which *was* computable — is not recoverable from the
        // error, because a partial vector is not a lesser prediction, it is a
        // different one.
        let Err(missing) = out else { unreachable!() };
        assert!(missing.iter().all(|m| !matches!(
            m,
            MissingFeature::UnknownToTheEngine { name } if name == "close"
        )));
    }

    #[test]
    fn no_rows_refuses_every_column_rather_than_claiming_they_are_unknown() {
        // Nothing was read, so nothing about the features has been established.
        // Reporting them as unknown-to-the-engine would send someone to look for
        // features that are fine.
        let missing = assemble(&names(&["close", "ema_5"]), &[]).expect_err("no bars");
        assert_eq!(missing.len(), 2);
        assert!(missing
            .iter()
            .all(|m| matches!(m, MissingFeature::NoValueAtAsOf { .. })));
    }

    #[test]
    fn an_abstention_does_not_serialise_as_a_prediction() {
        // The agent reads this JSON. An abstention that carried a `direction` and
        // a `confidence` — even null ones — would be quoted in a finding as
        // though the model had answered.
        let v = serde_json::to_value(ModelCall::Abstained {
            model_id: "mdl_1".into(),
            version: 3,
            detail: "circuit open".into(),
        })
        .expect("serialises");

        assert_eq!(v["outcome"], "abstained");
        assert!(v.get("direction").is_none(), "{v}");
        assert!(v.get("confidence").is_none(), "{v}");
        assert!(v.get("calibrated").is_none(), "{v}");
    }

    #[test]
    fn a_features_unavailable_outcome_carries_the_declared_list_beside_the_failures() {
        // "Three missing" means something different out of four columns than out
        // of forty, and the agent cannot tell which without the denominator.
        let v = serde_json::to_value(ModelCall::FeaturesUnavailable {
            model_id: "mdl_1".into(),
            version: 1,
            required: names(&["a", "b", "c", "d"]),
            missing: vec![MissingFeature::UnknownToTheEngine { name: "a".into() }],
        })
        .expect("serialises");

        assert_eq!(v["outcome"], "features_unavailable");
        assert_eq!(v["required"].as_array().map(Vec::len), Some(4));
        assert_eq!(v["missing"][0]["why"], "unknown_to_the_engine");
        assert_eq!(v["missing"][0]["name"], "a");
    }
}
