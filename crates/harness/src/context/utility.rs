//! Utility attribution (AGENT-005 §19, §28.2; Set S task S-9.10).
//!
//! # What this measures, and why it is the first real evidence
//!
//! This platform has never had evidence about what belongs in a prompt. Every
//! layout decision, every share, every "the model needs to see this" has been an
//! argument. Attribution replaces the argument with a count: after each call,
//! every rendered record is checked against what the model did next.
//!
//! Per run it is noise. Aggregated per `(GoalKind, RecordKind, signal bucket)`
//! across the scorecard corpus it is the thing S-9.12 fits the selection weights
//! to — so the weights stop being asserted in source and start being measured.
//!
//! It runs **offline over recorded trajectories at zero model cost**, which is
//! the payoff of the sans-IO loop: the tape already holds what was rendered and
//! what was called next, so there is nothing left to ask a provider.
//!
//! # `unused` and `dismissed` are not the same, and the metric says so
//!
//! §28.2's rule. "Rendered and not cited" covers two opposite outcomes: the model
//! never read it (a selection failure) and the model read it and correctly
//! decided it did not help (a selection success). They look identical in the
//! trace.
//!
//! Where the evidence cannot separate them this reports
//! [`Utility::Indeterminate`] rather than the flattering reading. That costs
//! precision and is the right trade: a metric that counted every dismissal as a
//! success would report a rising `utility_rate` for a selector that was getting
//! worse, and nobody would find out.

use std::collections::{BTreeMap, BTreeSet};

use super::store::{RecordId, RunStore, Utility};

/// One thing the model did after a render.
#[derive(Debug, Clone)]
pub struct Action {
    pub step: u32,
    pub tool: String,
    /// Every string value in the call's arguments, plus any evidence it cited.
    /// Flattened by the caller because a nested shape is the tape's business and
    /// not this module's.
    pub mentions: BTreeSet<String>,
    /// Records this action asked to `expand`.
    pub expanded: Vec<RecordId>,
}

impl Action {
    #[must_use]
    pub fn new(step: u32, tool: impl Into<String>) -> Self {
        Self {
            step,
            tool: tool.into(),
            mentions: BTreeSet::new(),
            expanded: Vec::new(),
        }
    }

    #[must_use]
    pub fn mentioning<I, S>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.mentions.extend(values.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn expanding(mut self, id: RecordId) -> Self {
        self.expanded.push(id);
        self
    }
}

/// What one assembly rendered.
#[derive(Debug, Clone)]
pub struct Render {
    pub step: u32,
    pub ids: Vec<RecordId>,
}

/// The attribution over one run.
#[derive(Debug, Clone, Default)]
pub struct Attribution {
    pub per_record: BTreeMap<RecordId, Utility>,
    /// Used tokens ÷ rendered tokens. Joins the scorecard per tier beside cache
    /// hit.
    pub utility_rate: f32,
    /// Share of rendered tokens the evidence could not classify. Reported beside
    /// the rate, because a rate computed over mostly-indeterminate records is a
    /// number with an error bar the size of itself.
    pub indeterminate_share: f32,
    pub rendered_tokens: u64,
    pub used_tokens: u64,
}

/// Attribute every rendered record against what the model did next (§19).
///
/// `renders` and `actions` come from one run's tape. An action at step *n* is
/// evidence about everything rendered at step *n* or earlier — a record can be
/// used two steps after it was shown, and scoring only the immediately following
/// call would report the long-range case this whole phase exists for as unused.
#[must_use]
pub fn attribute(store: &RunStore, renders: &[Render], actions: &[Action]) -> Attribution {
    let mut out = Attribution::default();
    let mut rendered_at: BTreeMap<RecordId, u32> = BTreeMap::new();
    for r in renders {
        for id in &r.ids {
            rendered_at.entry(*id).or_insert(r.step);
        }
    }

    // Did the run cite *anything*? If not, nothing can be told apart: a run that
    // referenced no record gives no evidence about which records it read.
    let cited_anything = actions.iter().any(|a| {
        !a.expanded.is_empty()
            || store
                .all()
                .any(|r| r.ids.values().any(|v| a.mentions.contains(v)))
    });

    for (id, first_step) in &rendered_at {
        let Some(record) = store.get(*id) else {
            continue;
        };
        let later: Vec<&Action> = actions.iter().filter(|a| a.step >= *first_step).collect();

        let expanded = later.iter().any(|a| a.expanded.contains(id));
        let cited = later.iter().any(|a| {
            record.ids.values().any(|v| a.mentions.contains(v))
                || a.mentions.contains(&id.to_string())
        });
        // The same tool called again with arguments derived from this record —
        // §19 counts that as used, because the second `get_bars` call wanting
        // the first one's shape is exactly what "used" means for a shape.
        let derived = later.iter().any(|a| {
            a.tool == record.source && !record.subject.is_empty() && {
                let label = record.subject.label();
                a.mentions.iter().any(|m| label.contains(m.as_str()))
            }
        });

        let verdict = if cited || derived {
            Utility::Used
        } else if expanded {
            // Wanted, and we under-rendered it.
            Utility::Wanted
        } else if !cited_anything {
            Utility::Indeterminate
        } else if engaged_with_subject(record, &later) {
            // The model worked in this record's area and cited something else.
            // That is "checked and dismissed" as easily as "never read", and the
            // trace cannot tell which.
            Utility::Indeterminate
        } else {
            Utility::Unused
        };
        out.per_record.insert(*id, verdict);

        let tokens = u64::from(record.tokens_full);
        out.rendered_tokens += tokens;
        match verdict {
            Utility::Used => out.used_tokens += tokens,
            Utility::Indeterminate => out.indeterminate_share += tokens as f32,
            _ => {}
        }
    }

    if out.rendered_tokens > 0 {
        out.utility_rate = out.used_tokens as f32 / out.rendered_tokens as f32;
        out.indeterminate_share /= out.rendered_tokens as f32;
    }
    out
}

/// Whether the model worked in this record's subject area after seeing it.
fn engaged_with_subject(record: &super::store::Record, later: &[&Action]) -> bool {
    if record.subject.is_empty() {
        return false;
    }
    let label = record.subject.label();
    later
        .iter()
        .any(|a| a.mentions.iter().any(|m| label.contains(m.as_str())))
}

/// `utility_rate` aggregated over a corpus, per bucket (§19).
///
/// The bucket key is the caller's — `(GoalKind, RecordKind, signal bucket)` is
/// what §19 names, and building it here would fix the dimensions before anything
/// has been measured.
#[must_use]
pub fn aggregate(runs: &[(String, Attribution)]) -> BTreeMap<String, (f32, u64)> {
    let mut sums: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for (bucket, a) in runs {
        let e = sums.entry(bucket.clone()).or_insert((0, 0));
        e.0 += a.used_tokens;
        e.1 += a.rendered_tokens;
    }
    sums.into_iter()
        .map(|(k, (used, rendered))| {
            let rate = if rendered == 0 {
                0.0
            } else {
                used as f32 / rendered as f32
            };
            (k, (rate, rendered))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::body::Body;
    use crate::context::store::{Draft, RecordKind, Subject};

    fn store() -> (RunStore, RecordId, RecordId, RecordId) {
        let mut s = RunStore::new();
        let cited = s.insert(
            Draft::new(
                1,
                RecordKind::ToolResult,
                "run_backtest",
                Body::text("x".repeat(400)),
            )
            .with_subject(Subject::instrument("BTC-USD"))
            .with_id("experiment_id", "exp_7"),
        );
        let nearby = s.insert(
            Draft::new(
                1,
                RecordKind::ToolResult,
                "get_bars",
                Body::text("y".repeat(400)),
            )
            .with_subject(Subject::instrument("BTC-USD"))
            .with_id("window", "w1"),
        );
        let elsewhere = s.insert(
            Draft::new(
                1,
                RecordKind::ToolResult,
                "get_bars",
                Body::text("z".repeat(400)),
            )
            .with_subject(Subject::instrument("DOGE-USD"))
            .with_id("window", "w2"),
        );
        (s, cited, nearby, elsewhere)
    }

    fn renders(ids: &[RecordId]) -> Vec<Render> {
        vec![Render {
            step: 2,
            ids: ids.to_vec(),
        }]
    }

    /// An id in the next call is the clearest signal there is.
    #[test]
    fn a_record_whose_id_is_used_next_is_used() {
        let (s, cited, _, _) = store();
        let a = attribute(
            &s,
            &renders(&[cited]),
            &[Action::new(3, "compare_backtests").mentioning(["exp_7"])],
        );
        assert_eq!(a.per_record[&cited], Utility::Used);
        assert!((a.utility_rate - 1.0).abs() < f32::EPSILON);
    }

    /// An `expand` means wanted, not used: we under-rendered it.
    #[test]
    fn an_expansion_is_a_signal_about_the_harness_not_the_record() {
        let (s, cited, _, _) = store();
        let a = attribute(
            &s,
            &renders(&[cited]),
            &[
                Action::new(3, "expand").expanding(cited),
                // Something else was cited, so the run is not blank.
                Action::new(4, "get_bars").mentioning(["w2"]),
            ],
        );
        assert_eq!(a.per_record[&cited], Utility::Wanted);
    }

    /// §28.2: the two readings of "rendered, not cited" are kept apart.
    #[test]
    fn checked_and_dismissed_is_not_counted_as_unused() {
        let (s, cited, nearby, elsewhere) = store();
        let a = attribute(
            &s,
            &renders(&[cited, nearby, elsewhere]),
            &[Action::new(3, "compare_backtests").mentioning(["exp_7", "BTC-USD"])],
        );
        assert_eq!(a.per_record[&cited], Utility::Used);
        assert_eq!(
            a.per_record[&nearby],
            Utility::Indeterminate,
            "the model worked on BTC-USD and cited something else: that is as easily a \
             dismissal as a miss, and the trace cannot tell"
        );
        assert_eq!(
            a.per_record[&elsewhere],
            Utility::Unused,
            "nothing the run did afterwards touched DOGE-USD"
        );
    }

    /// A run that cited nothing gives no evidence about anything.
    ///
    /// The flattering alternative is to mark every record unused and report a
    /// utility rate of zero, which reads as a selection failure when it is an
    /// absence of measurement.
    #[test]
    fn a_run_that_cited_nothing_is_indeterminate_throughout() {
        let (s, cited, nearby, elsewhere) = store();
        let a = attribute(
            &s,
            &renders(&[cited, nearby, elsewhere]),
            &[Action::new(3, "finish_task")],
        );
        for id in [cited, nearby, elsewhere] {
            assert_eq!(a.per_record[&id], Utility::Indeterminate);
        }
        assert!(
            (a.indeterminate_share - 1.0).abs() < f32::EPSILON,
            "and the share is reported, because a rate over mostly-indeterminate records \
             has an error bar the size of itself"
        );
    }

    /// A record can be used two steps after it was shown.
    ///
    /// Scoring only the immediately following call would report the long-range
    /// case this whole phase exists for as unused — which would make the metric
    /// argue against the mechanism it is measuring.
    #[test]
    fn a_record_used_three_steps_later_is_still_used() {
        let (s, cited, _, _) = store();
        let a = attribute(
            &s,
            &renders(&[cited]),
            &[
                Action::new(3, "get_bars").mentioning(["w9"]),
                Action::new(4, "reflect"),
                Action::new(5, "compare_backtests").mentioning(["exp_7"]),
            ],
        );
        assert_eq!(a.per_record[&cited], Utility::Used);
    }

    /// Attribution is offline: it takes a tape and touches nothing else.
    #[test]
    fn attribution_needs_no_provider() {
        let (s, cited, nearby, elsewhere) = store();
        // Two runs, aggregated by bucket.
        let a = attribute(
            &s,
            &renders(&[cited, nearby]),
            &[Action::new(3, "x").mentioning(["exp_7", "w1"])],
        );
        let b = attribute(
            &s,
            &renders(&[elsewhere]),
            &[Action::new(3, "x").mentioning(["exp_7"])],
        );
        let agg = aggregate(&[
            ("research.ToolResult".into(), a),
            ("research.ToolResult".into(), b),
        ]);
        let (rate, rendered) = agg["research.ToolResult"];
        assert!(rendered > 0);
        assert!((0.0..=1.0).contains(&rate), "{rate}");
    }
}
