//! The synthetic venue's generator job (`simulate_paths`, DATA-005 §9, DA-13).
//!
//! Generates a seeded instrument, writes its bars to `market_bars_v2` with
//! `venue_id = 'synthetic'` so that every downstream reader — the Data API, the
//! cutoff, `data_qc`, the backtester — treats it exactly like a real one, and
//! records the catalogue entry in Postgres.
//!
//! The catalogue entry is split in two on purpose. `public_meta` is what any reader
//! may see: how many bars, over what span, at what timeframe. `params` and `truth`
//! are the answer key, and live behind the `evals.truth` scope that no agent session
//! can hold (migration 0039). A generator that told the agent it had planted an
//! AR(1) with phi=0.05 would still produce a beautiful power curve; it just would
//! not be measuring the agent.

use backtest::synthetic::{GenParams, Generator, SyntheticSpec};
use chrono::{DateTime, TimeZone, Utc};
use jobs::{Cost, JobContext, JobError, JobKind, JobOutput, Progress, Worker};
use serde_json::{json, Value};

pub struct SimulatePathsWorker {
    clickhouse_url: String,
    pg: sqlx::PgPool,
}

impl SimulatePathsWorker {
    pub fn new(clickhouse_url: String, pg: sqlx::PgPool) -> Self {
        Self { clickhouse_url, pg }
    }

    /// Claims `SYN-<generator>-<seed>` for this exact spec, or refuses.
    ///
    /// Three outcomes, and the third is the one that matters:
    ///
    /// - the id is free: it is registered and the job continues;
    /// - the id is taken by an *identical* spec: a no-op, so a re-queued job after a
    ///   lost lease is safe (the generator is deterministic, so re-writing the same
    ///   bars is idempotent too);
    /// - the id is taken by a *different* spec: the job fails here, before a single
    ///   bar is written. The id has no room for the parameters, so `phi=0.02` and
    ///   `phi=0.10` on seed 3 are the same instrument, and interleaving their bars
    ///   would produce a series matching neither — with every scorecard that cited
    ///   it silently wrong. The fix is to vary the seed, which is what a task file
    ///   does anyway.
    async fn claim_instrument(
        &self,
        spec: &SyntheticSpec,
        truth: &backtest::synthetic::Truth,
        params_json: &Value,
        public_meta: &Value,
    ) -> Result<(), JobError> {
        let instrument_id = spec.instrument_id();
        let length = i64::try_from(spec.length).unwrap_or(i64::MAX);

        let inserted = sqlx::query_scalar::<_, String>(
            "INSERT INTO synthetic_instruments \
               (instrument_id, generator, seed, timeframe, length, start_time, \
                params, truth, public_meta) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (instrument_id) DO NOTHING \
             RETURNING instrument_id",
        )
        .bind(&instrument_id)
        .bind(spec.generator.as_str())
        .bind(i64::try_from(spec.seed).unwrap_or(i64::MAX))
        .bind(&spec.timeframe)
        .bind(length)
        .bind(spec.start)
        .bind(params_json)
        .bind(serde_json::to_value(truth).unwrap_or(Value::Null))
        .bind(public_meta)
        .fetch_optional(&self.pg)
        .await
        .map_err(|e| JobError::infrastructure(format!("registering the instrument failed: {e}")))?;

        if inserted.is_some() {
            return Ok(());
        }

        let existing: Option<(String, String, i64, Value, DateTime<Utc>)> = sqlx::query_as(
            "SELECT generator, timeframe, length, params, start_time \
               FROM synthetic_instruments WHERE instrument_id = $1",
        )
        .bind(&instrument_id)
        .fetch_optional(&self.pg)
        .await
        .map_err(|e| JobError::infrastructure(format!("reading the instrument failed: {e}")))?;

        let Some((generator, timeframe, existing_len, existing_params, start)) = existing else {
            // Lost the race and the row vanished — a retry will settle it.
            return Err(JobError::infrastructure(
                "the instrument row disappeared between insert and read",
            ));
        };

        let same = generator == spec.generator.as_str()
            && timeframe == spec.timeframe
            && existing_len == length
            && &existing_params == params_json
            && start == spec.start;

        if same {
            Ok(())
        } else {
            Err(JobError {
                code: "instrument_id_collision".into(),
                terminal: ledger::TerminalReason::IntegrityRejected,
                field: Some("seed".into()),
                rule: None,
                fix: Some(format!(
                    "{instrument_id} already exists with different parameters; the id is \
                     SYN-<generator>-<seed> and carries no room for params, so use a \
                     different seed for a different parameterisation"
                )),
                detail_ref: None,
                retryable: false,
            })
        }
    }
}

/// The default first bar for synthetic instruments: 2020-01-01T00:00:00Z.
///
/// Fixed rather than relative ("now minus length"), because a task whose window
/// moved every run would not be reproducible from its seed — and reproducibility
/// from the seed is the one property the whole suite rests on.
#[must_use]
pub fn default_synthetic_epoch() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0)
        .single()
        .expect("2020-01-01T00:00:00Z is a real instant")
}

/// Parses a `simulate_paths` manifest into a generator spec.
///
/// Accepts `tf` (the DATA-005 §9 request field) or `timeframe` (the name the rest
/// of the job manifests use). Getting that wrong costs a whole job round-trip to
/// discover, which is a poor way to learn a synonym.
#[allow(clippy::result_large_err)] // JobError is the job service's error type; boxing it here
                                   // would make every worker unbox it back.
pub fn parse_synthetic_manifest(manifest: &Value) -> Result<SyntheticSpec, JobError> {
    let generator_name = manifest
        .get("generator")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            JobError::logic(
                ledger::TerminalReason::IntegrityRejected,
                "invalid_manifest",
                "simulate_paths needs a generator in its manifest",
            )
        })?;
    let generator = Generator::parse(generator_name).ok_or_else(|| JobError {
        code: "unknown_generator".into(),
        terminal: ledger::TerminalReason::IntegrityRejected,
        field: Some("generator".into()),
        rule: None,
        fix: Some(format!(
            "known generators: {}",
            Generator::ALL
                .iter()
                .map(|g| g.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        detail_ref: None,
        retryable: false,
    })?;

    let params: GenParams = match manifest.get("params") {
        Some(v) if !v.is_null() => serde_json::from_value(v.clone()).map_err(|e| JobError {
            code: "invalid_params".into(),
            terminal: ledger::TerminalReason::IntegrityRejected,
            field: Some("params".into()),
            rule: None,
            fix: Some(format!("{e}")),
            detail_ref: None,
            retryable: false,
        })?,
        _ => GenParams::default(),
    };

    let seed = manifest
        .get("seed")
        .and_then(Value::as_u64)
        .ok_or_else(|| JobError::logic(ledger::TerminalReason::IntegrityRejected, "invalid_manifest", "simulate_paths needs a seed"))?;

    let timeframe = manifest
        .get("tf")
        .or_else(|| manifest.get("timeframe"))
        .and_then(Value::as_str)
        .unwrap_or("1h")
        .to_string();

    let length_raw = manifest
        .get("length")
        .and_then(Value::as_u64)
        .ok_or_else(|| JobError::logic(ledger::TerminalReason::IntegrityRejected, "invalid_manifest", "simulate_paths needs a length"))?;
    let length = usize::try_from(length_raw)
        .map_err(|_| JobError::logic(ledger::TerminalReason::IntegrityRejected, "invalid_manifest", "length does not fit in memory"))?;

    let start = match manifest.get("start") {
        Some(v) if !v.is_null() => {
            serde_json::from_value::<DateTime<Utc>>(v.clone()).map_err(|e| JobError {
                code: "invalid_start".into(),
                terminal: ledger::TerminalReason::IntegrityRejected,
                field: Some("start".into()),
                rule: None,
                fix: Some(format!("use an RFC 3339 instant: {e}")),
                detail_ref: None,
                retryable: false,
            })?
        }
        _ => default_synthetic_epoch(),
    };

    Ok(SyntheticSpec {
        generator,
        params,
        seed,
        timeframe,
        length,
        start,
    })
}

#[async_trait::async_trait]
impl Worker for SimulatePathsWorker {
    fn kind(&self) -> JobKind {
        JobKind::SimulatePaths
    }

    fn estimate(&self, manifest: &Value) -> Cost {
        let length = manifest.get("length").and_then(Value::as_u64).unwrap_or(0);
        // Generation is microseconds per bar; the insert dominates.
        Cost {
            compute_s: Some(1.0 + length as f64 / 20_000.0),
            gpu_s: None,
            cost_usd: Some(0.0),
        }
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let spec = parse_synthetic_manifest(manifest)?;
        let instrument_id = spec.instrument_id();

        ctx.progress(Progress {
            pct: Some(0.1),
            stage: Some("generating".into()),
            message: Some(format!("{} seed {}", spec.generator, spec.seed)),
        })
        .await;

        let (bars, truth) = backtest::synthetic::generate(&spec).map_err(|e| JobError {
            code: "generator_refused".into(),
            terminal: ledger::TerminalReason::IntegrityRejected,
            field: None,
            rule: None,
            fix: Some(e.to_string()),
            detail_ref: None,
            retryable: false,
        })?;
        let timeframe = spec.resolve_timeframe().map_err(|e| JobError {
            code: "unknown_timeframe".into(),
            terminal: ledger::TerminalReason::IntegrityRejected,
            field: Some("tf".into()),
            rule: None,
            fix: Some(e.to_string()),
            detail_ref: None,
            retryable: false,
        })?;

        let first = bars.first().map(|b| b.available_time);
        let last = bars.last().map(|b| b.available_time);
        let public_meta = json!({
            "instrument_id": instrument_id,
            "venue_id": "synthetic",
            "timeframe": spec.timeframe,
            "bars": bars.len(),
            "first_event_time": first,
            "last_event_time": last,
        });
        let params_json = serde_json::to_value(&spec.params).unwrap_or(Value::Null);

        ctx.progress(Progress {
            pct: Some(0.4),
            stage: Some("registering".into()),
            message: None,
        })
        .await;

        // Registration happens *before* the bars are written, and this is the whole
        // reason: the instrument id is `SYN-<generator>-<seed>` (DATA-005 §9), so two
        // requests differing only in `params` claim the same id. Writing first would
        // mix two series into one instrument and every scorecard citing it would be
        // quietly wrong. Claiming the id first turns that into an error.
        self.claim_instrument(&spec, &truth, &params_json, &public_meta)
            .await?;

        ctx.progress(Progress {
            pct: Some(0.6),
            stage: Some("writing".into()),
            message: Some(format!("{} bars", bars.len())),
        })
        .await;

        let store = backtest::store::BarStore::connect(&self.clickhouse_url);
        store
            .insert_collected(
                &instrument_id,
                "synthetic",
                "synthetic",
                // Synthetic bars are exactly as trustworthy as their generator, which
                // is to say completely — and labelling them anything else would let a
                // grade-based gate silently refuse the eval suite's own data.
                "reference",
                timeframe,
                &bars,
            )
            .await
            .map_err(|e| JobError::infrastructure(format!("writing synthetic bars failed: {e}")))?;

        // The summary is read by an agent. It says what was made and how much of it,
        // and says nothing whatsoever about what is in it.
        let summary = format!(
            "{instrument_id}: {} synthetic {} bars written to the synthetic venue",
            bars.len(),
            spec.timeframe
        );

        Ok(JobOutput {
            summary: Some(summary),
            result: public_meta,
            artifacts: vec![],
            actual: Cost {
                compute_s: Some(1.0 + bars.len() as f64 / 20_000.0),
                gpu_s: None,
                cost_usd: Some(0.0),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimal_manifest_is_enough() {
        let spec = parse_synthetic_manifest(&json!({
            "generator": "garch_t",
            "seed": 7,
            "length": 500,
        }))
        .unwrap();
        assert_eq!(spec.generator, Generator::GarchT);
        assert_eq!(spec.timeframe, "1h");
        assert_eq!(spec.start, default_synthetic_epoch());
        assert_eq!(spec.instrument_id(), "SYN-GARCH-T-7");
    }

    #[test]
    fn tf_and_timeframe_are_the_same_field() {
        let a = parse_synthetic_manifest(&json!({
            "generator": "garch_t", "seed": 1, "length": 10, "tf": "15m"
        }))
        .unwrap();
        let b = parse_synthetic_manifest(&json!({
            "generator": "garch_t", "seed": 1, "length": 10, "timeframe": "15m"
        }))
        .unwrap();
        assert_eq!(a.timeframe, b.timeframe);
    }

    #[test]
    fn an_unknown_generator_names_the_known_ones() {
        let err = parse_synthetic_manifest(&json!({
            "generator": "brownian", "seed": 1, "length": 10
        }))
        .unwrap_err();
        assert_eq!(err.code, "unknown_generator");
        let fix = err.fix.unwrap();
        assert!(
            fix.contains("garch_t"),
            "the fix must list what is available"
        );
        assert!(fix.contains("planted_carry"));
    }

    /// The default epoch must not move with the wall clock, or a task file stops
    /// being reproducible from its seed.
    #[test]
    fn the_default_start_is_a_fixed_instant() {
        assert_eq!(
            default_synthetic_epoch().to_rfc3339(),
            "2020-01-01T00:00:00+00:00"
        );
    }

    /// The manifest is hashed for idempotency, so two spellings of the same request
    /// must not silently produce two instruments with the same id.
    #[test]
    fn the_instrument_id_is_a_function_of_generator_and_seed_only() {
        let a = parse_synthetic_manifest(&json!({
            "generator": "planted_ar1", "seed": 3, "length": 10,
            "params": {"phi": 0.02}
        }))
        .unwrap();
        let b = parse_synthetic_manifest(&json!({
            "generator": "planted_ar1", "seed": 3, "length": 10,
            "params": {"phi": 0.10}
        }))
        .unwrap();
        // Same id, different series. `claim_instrument` turns that into an
        // `instrument_id_collision` before a bar is written, instead of a silently
        // wrong scorecard — so the seed, not the params, is what a task file varies.
        assert_eq!(a.instrument_id(), b.instrument_id());
        let ra = backtest::synthetic::returns(&a).unwrap().0;
        let rb = backtest::synthetic::returns(&b).unwrap().0;
        assert_ne!(ra, rb);
    }
}
