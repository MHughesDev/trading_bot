//! Barren rate per `(tier, workflow)`, from the runs that actually happened
//! (AGENT-013 §10; AGENT-009 §4 amended; Set S task S-10.13).
//!
//! # What this measures that nothing else does
//!
//! Every other number the platform keeps about a run says whether the harness
//! *worked*: did it finish, did it fence, how many steps, what did it cost. A
//! barren run finished perfectly. It spent from the trial budget — the one
//! quantity every selection-adjusted statistic on this platform depends on — and
//! produced nothing the next run can use.
//!
//! `Yield::is_barren` decides it per run and `agent_runs.barren` stores it.
//! `barren_rates` aggregates it. Neither had a caller over real rows, so the
//! flag was computed on every run and read by nothing.
//!
//! # Never one number
//!
//! Keyed by tier *and* workflow, and never averaged across them. "12% barren" is
//! something nobody can act on; "`survey` on the 7B is barren 60% of the time"
//! names a tier and a workflow, so somebody can change one of them. The same
//! argument AGENT-009 §4 makes about pass rates, one level up — and the reason
//! `agent_runs_barren_idx` is keyed the way it is.
//!
//! # Unresolvable profiles are reported, not merged
//!
//! The tier comes from the profile, and a run may name a profile the platform no
//! longer ships. Those rows are returned separately rather than being folded
//! into a guessed tier or dropped: folding them would attribute one tier's waste
//! to another, and dropping them would make the platform look less wasteful the
//! more configurations it had retired.

use harness::lifecycle::r#yield::BarrenRate;
use harness::ProfileSet;
use sqlx::PgPool;

/// The rates, and what could not be attributed to a tier.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rates {
    pub rows: Vec<BarrenRate>,
    /// Profile ids no longer in the shipped set, with their run counts. Named
    /// rather than dropped.
    pub unresolved: Vec<(String, u32)>,
}

impl Rates {
    /// Total runs counted, resolved or not.
    #[must_use]
    pub fn runs(&self) -> u32 {
        self.rows.iter().map(|r| r.runs).sum::<u32>()
            + self.unresolved.iter().map(|(_, n)| n).sum::<u32>()
    }
}

/// Barren rates over finished runs, grouped by `(tier, workflow)`.
///
/// `days` bounds the window. A rate over all time answers a question about a
/// platform that no longer exists — the harness, the profiles and the workflows
/// have all changed — and the number somebody can act on is the recent one.
///
/// Runs still going are excluded by `barren IS NOT NULL`: a run that has not
/// finished is not barren, it is unfinished, and counting it as productive or as
/// waste would both be claims about a run that has not made one yet.
///
/// # Errors
/// The database failed.
pub async fn rates(pg: &PgPool, profiles: &ProfileSet, days: i32) -> Result<Rates, sqlx::Error> {
    let rows: Vec<(Option<String>, Option<String>, i64, i64)> = sqlx::query_as(
        "SELECT profile_name, workflow, count(*), count(*) FILTER (WHERE barren)
           FROM agent_runs
          WHERE barren IS NOT NULL
            AND finished_at >= now() - make_interval(days => $1)
          GROUP BY profile_name, workflow",
    )
    .bind(days)
    .fetch_all(pg)
    .await?;

    let mut out = Rates::default();
    // Tier and workflow together, summed across the profiles that share a tier:
    // two 7B profiles are one tier's story, and reporting them apart would split
    // a rate somebody is trying to read in half.
    let mut by_tier: std::collections::BTreeMap<(String, String), (u32, u32)> =
        std::collections::BTreeMap::new();

    for (profile_name, workflow, runs, barren) in rows {
        let runs = u32::try_from(runs).unwrap_or(u32::MAX);
        let barren = u32::try_from(barren).unwrap_or(u32::MAX);
        let Some(name) = profile_name else {
            out.unresolved
                .push((String::from("(no profile recorded)"), runs));
            continue;
        };
        let Ok(profile) = profiles.resolve(&name) else {
            out.unresolved.push((name, runs));
            continue;
        };
        // A run with no workflow recorded predates S-10.1's column. Named as
        // such rather than counted under `investigate`, which is the default and
        // would make the default look worse than it is.
        let workflow = workflow.unwrap_or_else(|| "(not recorded)".to_string());
        let e = by_tier
            .entry((profile.tier.as_str().to_string(), workflow))
            .or_insert((0, 0));
        e.0 = e.0.saturating_add(runs);
        e.1 = e.1.saturating_add(barren);
    }

    out.rows = by_tier
        .into_iter()
        .map(|((tier, workflow), (runs, barren))| BarrenRate {
            tier,
            workflow,
            runs,
            barren,
        })
        .collect();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(tier: &str, workflow: &str, runs: u32, barren: u32) -> BarrenRate {
        BarrenRate {
            tier: tier.into(),
            workflow: workflow.into(),
            runs,
            barren,
        }
    }

    /// An empty result is not a zero rate.
    ///
    /// `BarrenRate::rate` returns `None` with no runs, so a renderer cannot show
    /// a platform that has produced nothing at all as one that never wastes a
    /// trial.
    #[test]
    fn no_runs_is_not_a_rate_of_zero() {
        assert_eq!(rate("frontier", "investigate", 0, 0).rate(), None);
        assert_eq!(Rates::default().runs(), 0);
    }

    /// Unresolved profiles count toward the total, so the two halves cannot
    /// silently disagree about how many runs there were.
    #[test]
    fn unresolved_runs_are_still_counted() {
        let r = Rates {
            rows: vec![rate("frontier", "investigate", 8, 2)],
            unresolved: vec![("qwen2.5:3b-retired".into(), 5)],
        };
        assert_eq!(r.runs(), 13);
        assert_eq!(r.rows[0].rate(), Some(0.25));
    }

    /// The rate is per row and never summed into one number by this module.
    ///
    /// Two tiers with very different rates average to something true of neither,
    /// and the average is what a caller reaches for when it is offered.
    #[test]
    fn two_tiers_are_two_rows() {
        let r = Rates {
            rows: vec![
                rate("degraded", "survey", 10, 6),
                rate("frontier", "survey", 10, 0),
            ],
            unresolved: vec![],
        };
        assert_eq!(r.rows[0].rate(), Some(0.6));
        assert_eq!(r.rows[1].rate(), Some(0.0));
        // And a caller that wants one anyway gets it from the scorecard, where
        // the doc comment says not to show it.
        assert_eq!(r.runs(), 20);
    }
}
