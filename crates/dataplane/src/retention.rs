//! Disk budgets, downsampling, and the silence alarm for forward-only series
//! (Set P, P-0.6).
//!
//! Three things that had to exist *before* the first snapshot, not after.
//!
//! **Silence is this phase's real failure mode.** A collector that dies quietly
//! costs exactly what never collecting it would have, and it costs it invisibly:
//! there is no error, no failed job, no alert — just a series that stops growing
//! while everything downstream keeps reporting healthy. Under a free-only data
//! policy the hole it leaves cannot be filled afterwards at any price
//! (`artifact.md` FM-11). So the alarm is an acceptance criterion, not a
//! nice-to-have, and it is driven by a clock the tests advance by hand rather than
//! by sleeping.
//!
//! **Nothing is ever deleted.** [`RetentionAction`] has no delete variant, and that
//! is the point rather than an omission. When a series approaches its disk budget
//! it is *downsampled* — a coarser row replaces a finer one and the series stays
//! continuous. Deleting old rows to free space would destroy the only copy that
//! will ever exist, which is the trade this whole phase exists to refuse. A delete
//! tier that is "only for old data" is still a switch someone flips at 3 a.m. when
//! the disk fills.
//!
//! Pure logic. The Postgres side is `dataplane.forward_series` and
//! `dataplane.forward_series_alarm` (migration 0098), whose CHECK constraints
//! mirror the rules here so neither can drift alone.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// A forward series' declared cadence and budget.
///
/// Mirrors a row of `dataplane.forward_series`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardSeriesSpec {
    pub series_id: String,
    /// The ClickHouse table its observations land in.
    pub store_table: String,
    pub venue: String,
    /// How often a row is expected, in seconds.
    pub cadence_secs: u32,
    /// The alarm fires when the newest row is older than this many cadences.
    pub alarm_after_cadences: u32,
    pub disk_budget_bytes: u64,
    pub retention: RetentionPolicy,
    pub enabled: bool,
}

/// What happens when a series approaches its budget.
///
/// There is no `Delete`. See the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "policy", rename_all = "snake_case")]
pub enum RetentionPolicy {
    /// Small enough to keep at full resolution forever. Funding and open interest
    /// are a few hundred bytes an hour; they will not need a policy this decade.
    None,
    /// Beyond `after_days`, rows are coarsened to `to_secs` cadence. Kept, never
    /// removed.
    Downsample { after_days: u32, to_secs: u32 },
}

impl RetentionPolicy {
    /// A downsampling policy must coarsen, never refine, and must be complete.
    ///
    /// # Errors
    /// A policy that refines, or one whose target cadence is zero.
    pub fn validate(self, cadence_secs: u32) -> Result<(), RetentionError> {
        match self {
            Self::None => Ok(()),
            Self::Downsample { after_days, to_secs } => {
                if to_secs == 0 || after_days == 0 {
                    return Err(RetentionError::IncompletePolicy);
                }
                if to_secs < cadence_secs {
                    return Err(RetentionError::Refines { to_secs, cadence_secs });
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RetentionError {
    #[error("a downsampling policy must state both after_days and to_secs")]
    IncompletePolicy,
    #[error("downsampling to {to_secs}s would refine a {cadence_secs}s series, not coarsen it")]
    Refines { to_secs: u32, cadence_secs: u32 },
}

/// What the retention rule says to do right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RetentionAction {
    /// Inside budget. Do nothing.
    Keep,
    /// Over or near budget: coarsen rows older than `older_than_days` to `to_secs`.
    Downsample { older_than_days: u32, to_secs: u32 },
    /// Over budget with no policy that can help. Deliberately an *alert*, not a
    /// deletion: a human decides, because the alternative is a machine silently
    /// destroying an unrepeatable series to save disk.
    BudgetExceededNeedsDecision { used_bytes: u64, budget_bytes: u64 },
}

/// The share of budget at which downsampling begins. Acting at 100 % would mean
/// acting after the writes have already started failing.
pub const BUDGET_HIGH_WATER_PCT: u64 = 80;

/// Decide what to do about a series' disk use.
#[must_use]
pub fn retention_action(spec: &ForwardSeriesSpec, used_bytes: u64) -> RetentionAction {
    let high_water = spec.disk_budget_bytes.saturating_mul(BUDGET_HIGH_WATER_PCT) / 100;
    if used_bytes < high_water {
        return RetentionAction::Keep;
    }
    match spec.retention {
        RetentionPolicy::Downsample { after_days, to_secs } => {
            RetentionAction::Downsample { older_than_days: after_days, to_secs }
        }
        RetentionPolicy::None => {
            RetentionAction::BudgetExceededNeedsDecision { used_bytes, budget_bytes: spec.disk_budget_bytes }
        }
    }
}

/// Whether a forward series is still producing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlarmState {
    Ok,
    /// Producing once, now silent past its deadline.
    Stale,
    /// Registered as a promise and never produced a row at all. Distinct from
    /// `Stale` on purpose: a collector that never started and one that died look
    /// identical in a row count, and they need different fixes.
    NeverCollected,
}

/// The alarm's reading for one series.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alarm {
    pub state: AlarmState,
    /// Seconds past the deadline. Negative while healthy, so a dashboard can show
    /// headroom as easily as overdue.
    pub overdue_secs: i64,
    /// How much data a hole of this length represents, in expected rows. This is
    /// the number that makes silence concrete: "stale" is a status, "you have
    /// permanently lost 47 hourly funding observations" is a cost.
    pub rows_missed: u64,
}

impl Alarm {
    #[must_use]
    pub const fn is_firing(&self) -> bool {
        !matches!(self.state, AlarmState::Ok)
    }
}

/// Evaluate the silence alarm.
///
/// `last_row_at` is the newest observation's event time; `None` means the series
/// has never produced a row.
///
/// The deadline is `cadence * alarm_after_cadences` past the last row. A disabled
/// series never fires — a collector deliberately switched off is not a surprise —
/// but note that disabling one is still a decision to accept a permanent hole, and
/// `dataplane.forward_series.enabled` records who made it.
#[must_use]
pub fn alarm(spec: &ForwardSeriesSpec, last_row_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Alarm {
    let cadence = i64::from(spec.cadence_secs.max(1));
    let deadline_secs = cadence * i64::from(spec.alarm_after_cadences.max(1));

    let Some(last) = last_row_at else {
        return Alarm {
            state: if spec.enabled { AlarmState::NeverCollected } else { AlarmState::Ok },
            overdue_secs: 0,
            rows_missed: 0,
        };
    };

    let age = (now - last).num_seconds();
    let overdue_secs = age - deadline_secs;
    if !spec.enabled || overdue_secs <= 0 {
        return Alarm { state: AlarmState::Ok, overdue_secs, rows_missed: 0 };
    }
    // Rows that should have arrived since the last one and did not. The row at
    // `last` itself arrived, so the count is over the elapsed cadences beyond it.
    let rows_missed = u64::try_from(age / cadence).unwrap_or(0).saturating_sub(1);
    Alarm { state: AlarmState::Stale, overdue_secs, rows_missed }
}

/// Time until a healthy series would trip the alarm. `None` once it already has.
#[must_use]
pub fn time_to_alarm(spec: &ForwardSeriesSpec, last_row_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Option<Duration> {
    let a = alarm(spec, last_row_at, now);
    if a.is_firing() {
        return None;
    }
    last_row_at?;
    Some(Duration::seconds(-a.overdue_secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn hourly() -> ForwardSeriesSpec {
        ForwardSeriesSpec {
            series_id: "kraken_futures.funding".into(),
            store_table: "funding_rates".into(),
            venue: "kraken_futures".into(),
            cadence_secs: 3600,
            alarm_after_cadences: 3,
            disk_budget_bytes: 2_147_483_648,
            retention: RetentionPolicy::None,
            enabled: true,
        }
    }

    fn t(h: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 15, 0, 0, 0).unwrap() + Duration::hours(h)
    }

    /// The acceptance criterion: stopping a collector raises the alarm within *k*
    /// cadences, measured by advancing a clock rather than sleeping.
    #[test]
    fn stopping_a_collector_raises_the_alarm_within_k_cadences() {
        let spec = hourly();
        let last = t(0);
        // Healthy right up to the deadline.
        for h in 0..=3 {
            let a = alarm(&spec, Some(last), t(h));
            assert_eq!(a.state, AlarmState::Ok, "hour {h} is inside the {}-cadence deadline", spec.alarm_after_cadences);
            assert!(!a.is_firing());
        }
        // One second past it, it fires.
        let a = alarm(&spec, Some(last), t(3) + Duration::seconds(1));
        assert_eq!(a.state, AlarmState::Stale);
        assert_eq!(a.overdue_secs, 1);
        assert!(a.is_firing());
    }

    /// Silence is stated as a cost, not a status. A dashboard that says "stale"
    /// invites a shrug; one that says "you have permanently lost 47 observations"
    /// does not.
    #[test]
    fn the_alarm_counts_what_was_permanently_lost() {
        let spec = hourly();
        let a = alarm(&spec, Some(t(0)), t(48));
        assert_eq!(a.state, AlarmState::Stale);
        assert_eq!(a.rows_missed, 47, "47 hourly observations that no backfill can recover");
        assert_eq!(a.overdue_secs, (48 - 3) * 3600);
    }

    /// A collector that never started and one that died are different failures with
    /// different fixes, and a row count cannot tell them apart.
    #[test]
    fn never_collected_is_not_the_same_as_stale() {
        let spec = hourly();
        assert_eq!(alarm(&spec, None, t(100)).state, AlarmState::NeverCollected);
        assert_eq!(alarm(&spec, Some(t(0)), t(100)).state, AlarmState::Stale);
        // And it fires immediately rather than after a deadline it has no basis to
        // measure: there is no last row to count from.
        assert!(alarm(&spec, None, t(0)).is_firing());
    }

    #[test]
    fn a_disabled_series_does_not_fire() {
        let spec = ForwardSeriesSpec { enabled: false, ..hourly() };
        assert_eq!(alarm(&spec, Some(t(0)), t(1000)).state, AlarmState::Ok);
        assert_eq!(alarm(&spec, None, t(1000)).state, AlarmState::Ok);
    }

    #[test]
    fn headroom_is_reported_while_healthy() {
        let spec = hourly();
        let left = time_to_alarm(&spec, Some(t(0)), t(1)).unwrap();
        assert_eq!(left.num_hours(), 2, "three cadences of deadline, one hour elapsed");
        assert!(time_to_alarm(&spec, Some(t(0)), t(10)).is_none(), "already firing has no headroom");
    }

    // ── retention ────────────────────────────────────────────────────────────

    /// Adversarial, and the one that matters: the retention rule has no way to
    /// express deletion. Over budget with no downsampling policy, it asks a human —
    /// it does not free space by destroying an unrepeatable series.
    #[test]
    fn over_budget_never_deletes() {
        let spec = hourly();
        let action = retention_action(&spec, spec.disk_budget_bytes * 2);
        match action {
            RetentionAction::BudgetExceededNeedsDecision { used_bytes, budget_bytes } => {
                assert_eq!(budget_bytes, spec.disk_budget_bytes);
                assert!(used_bytes > budget_bytes);
            }
            other => panic!("over budget with no policy must escalate, not act: {other:?}"),
        }
        // The type itself admits no delete. If a `Delete` variant is ever added
        // this match stops compiling, which is the intended tripwire.
        match action {
            RetentionAction::Keep | RetentionAction::Downsample { .. } | RetentionAction::BudgetExceededNeedsDecision { .. } => {}
        }
    }

    #[test]
    fn downsampling_starts_at_the_high_water_mark_not_at_the_limit() {
        let spec = ForwardSeriesSpec {
            retention: RetentionPolicy::Downsample { after_days: 30, to_secs: 86_400 },
            disk_budget_bytes: 1_000,
            ..hourly()
        };
        assert_eq!(retention_action(&spec, 799), RetentionAction::Keep);
        assert_eq!(
            retention_action(&spec, 800),
            RetentionAction::Downsample { older_than_days: 30, to_secs: 86_400 },
            "acting only at 100% means acting after writes have already failed"
        );
    }

    /// Adversarial: a "downsampling" policy that refines is a typo that would
    /// silently do nothing, or worse, invent rows. It is refused.
    #[test]
    fn a_policy_that_refines_instead_of_coarsening_is_refused() {
        let cadence = 3600;
        let bad = RetentionPolicy::Downsample { after_days: 30, to_secs: 60 };
        assert_eq!(bad.validate(cadence), Err(RetentionError::Refines { to_secs: 60, cadence_secs: 3600 }));

        let good = RetentionPolicy::Downsample { after_days: 30, to_secs: 86_400 };
        assert!(good.validate(cadence).is_ok());
        // Equal is fine: a no-op coarsening is harmless.
        assert!(RetentionPolicy::Downsample { after_days: 1, to_secs: cadence }.validate(cadence).is_ok());
        assert!(RetentionPolicy::None.validate(cadence).is_ok());
    }

    #[test]
    fn an_incomplete_policy_is_refused() {
        assert_eq!(
            RetentionPolicy::Downsample { after_days: 0, to_secs: 86_400 }.validate(3600),
            Err(RetentionError::IncompletePolicy)
        );
        assert_eq!(
            RetentionPolicy::Downsample { after_days: 30, to_secs: 0 }.validate(3600),
            Err(RetentionError::IncompletePolicy)
        );
    }

    #[test]
    fn the_seeded_series_validate_against_their_own_cadence() {
        for spec in [hourly(), ForwardSeriesSpec { series_id: "kraken_futures.open_interest".into(), ..hourly() }] {
            assert!(spec.retention.validate(spec.cadence_secs).is_ok(), "{} has an invalid policy", spec.series_id);
        }
    }
}
