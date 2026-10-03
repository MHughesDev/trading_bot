-- Set U U-5.5: model prediction series (FEAT-006 §6, ADR-0043 decision 5).
--
-- Every answer a model version gave a backtest, keyed by exactly what it was
-- asked: the pinned version, the decision instant and a digest of the assembled
-- inputs. The backtest's point-in-time provider reads here before it calls the
-- inference sidecar and writes here after, and the `predict_series` job fills a
-- whole window ahead of time — so a backtest over a computed window, and every
-- repeat of any backtest, makes no sidecar calls.
--
-- Only answers the point-in-time rule allowed are ever written: a row exists for
-- a decision only if the version was trained before it minus its embargo. The
-- series therefore covers exactly the version's out-of-sample window.
--
-- `outcome` is the engine's `ModelOutcome` as JSON (answered outputs or an
-- abstain). ReplacingMergeTree on the key: recomputing a row replaces it rather
-- than duplicating it.

CREATE TABLE IF NOT EXISTS model_prediction_series (
    model_id            String,
    version             UInt32,
    at_ns               Int64,
    inputs_digest       String,
    status              LowCardinality(String),
    outcome             String,
    trained_through_ns  Nullable(Int64),
    written_at          DateTime64(3, 'UTC') DEFAULT now64(3)
)
ENGINE = ReplacingMergeTree(written_at)
ORDER BY (model_id, version, at_ns, inputs_digest);
