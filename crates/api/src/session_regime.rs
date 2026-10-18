//! Regime flips, as source 4.7 reads them (AGENT-014 §4.7; Set S task S-11.10).
//!
//! # What was missing
//!
//! `Observation::RegimeFlip` had no producer. The task said why in one line:
//! *"no worker claims `JobKind::InstrumentProfile`"*, and the regime had no
//! record either. Both moved. `instrument_profile_worker` now claims the job, and
//! `regime_causal.regime_state` holds a statistical label per market scope per
//! day (`low_vol | mid_vol | high_vol`, ADR-P3-02), written by the platform's
//! regime job and granted to `app_role` by 0166.
//!
//! # What a flip is here
//!
//! A change of `label` between consecutive rows of one scope under one
//! `model_version`. The partition is by version on purpose: a refit that
//! relabels history is a new model and not the market changing, and counting it
//! would raise an item per scope on the day somebody retrained.
//!
//! **`since` is the day the label changed**, the `event_time` of the first row
//! carrying the new label — not the day anyone looked. That is the whole of
//! §4.7's rate limit: *"one item per regime flip, not one per profiler run"*. The
//! fingerprint (`regime:{subject}:{from}->{to}@{since}`) is therefore the same on
//! every read of the same flip, and `Raised` — derived from the agenda's
//! `source_refs`, no fourth table — holds the second and third reads out. The
//! tests below read the same history three times and count one.
//!
//! The **subject** is the scope's `series_symbol` (the instrument whose series
//! *is* the market for that scope) and falls back to the scope's name when the
//! registry has no row, because the charter's scope is checked against
//! instruments and a flip in "the crypto market" is an event about BTC-USD.
//!
//! Only the filtered table is read. `regime_research.regime_state_smoothed` is the
//! answer computed with hindsight (AT-42): a flip read off it would be a flip the
//! session could not have seen the day it happened.

use research_session::sources::observe::Observation;
use sqlx::PgPool;

/// How far back flips are looked for. A flip older than this has had every
/// chance to be raised.
pub const FLIP_LOOKBACK_DAYS: i32 = 30;

/// The regime flips of the last [`FLIP_LOOKBACK_DAYS`], newest first.
///
/// # Errors
/// Database errors.
pub async fn flips(pg: &PgPool, limit: i64) -> Result<Vec<Observation>, sqlx::Error> {
    type Row = (String, String, String, String);
    let rows: Vec<Row> = sqlx::query_as(
        "WITH ordered AS (
             SELECT s.market_scope, s.event_time, s.label,
                    LAG(s.label) OVER (
                        PARTITION BY s.market_scope, s.model_version
                        ORDER BY s.event_time) AS prev
               FROM regime_causal.regime_state s
              WHERE s.event_time > now() - make_interval(days => $1::int)
         )
         SELECT COALESCE(NULLIF(btrim(r.series_symbol), ''), o.market_scope),
                o.prev, o.label,
                to_char(o.event_time AT TIME ZONE 'UTC', 'YYYY-MM-DD')
           FROM ordered o
           LEFT JOIN regime_causal.regime_scope r ON r.market_scope = o.market_scope
          WHERE o.prev IS NOT NULL AND o.prev <> o.label
          ORDER BY o.event_time DESC, o.market_scope
          LIMIT $2",
    )
    .bind(FLIP_LOOKBACK_DAYS)
    .bind(limit)
    .fetch_all(pg)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(subject, from, to, since)| Observation::RegimeFlip {
            subject,
            from,
            to,
            since,
        })
        .collect())
}
