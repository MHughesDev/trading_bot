//! Two memory rules that need no store to state (Set R-b RB-1.1, RB-1.2; KM-02, KM-03;
//! AGENT-003 §5).
//!
//! * **Retrieval metadata** ([`annotate`]): every retrieved row carries its age, a
//!   staleness flag and a capability-mismatch flag, so a reader sees how old a claim is
//!   and whether it presumes something this project cannot do before relying on it.
//! * **A finding is never evidence** ([`refuse_findings_as_evidence`]) and **a repeated
//!   hypothesis is a repeated trial** ([`RepeatKey`], [`flag_repeat`]). A hypothesis
//!   whose `source` is `memory:<finding>` on the same instrument and an overlapping
//!   window as an earlier one is not a new idea; counting it as one would deflate the
//!   trial count (INV-1) by exactly the amount memory made it easy to repeat.
//!
//! The store and the routes call these. Nothing here does I/O.

use serde::{Deserialize, Serialize};

use super::{days_between, DataWindow};

/// What a retrieved row says about itself, beyond its claim.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Annotation {
    /// Whole days since the claim was last confirmed (or created).
    pub age_days: i64,
    /// Older than the kind's validity, or already marked stale.
    pub stale: bool,
    /// `Some(true)` when the claim presumes a capability the caller lacks, `Some(false)`
    /// when it does not, and `None` when the caller did not say what it has. Unknown is
    /// not "fine".
    pub capability_mismatch: Option<bool>,
    /// The capabilities the claim presumes and the caller lacks.
    pub missing_capabilities: Vec<String>,
}

/// Annotate one row.
///
/// `last_confirmed` and `today` are ISO dates; `validity_days` is the kind's stale
/// horizon (`None` never expires on a timer). `requires` is what the claim presumes
/// (`scope.requires`); `have` is the caller's capabilities, `None` when unstated.
#[must_use]
pub fn annotate(
    last_confirmed: &str,
    today: &str,
    validity_days: Option<i64>,
    already_stale: bool,
    requires: &[String],
    have: Option<&[String]>,
) -> Annotation {
    let age_days = days_between(last_confirmed, today).unwrap_or(0).max(0);
    let stale = already_stale || validity_days.is_some_and(|v| age_days > v);
    let (capability_mismatch, missing) = match have {
        None => (None, Vec::new()),
        Some(have) => {
            let missing: Vec<String> = requires.iter().filter(|r| !have.contains(r)).cloned().collect();
            (Some(!missing.is_empty()), missing)
        }
    };
    Annotation {
        age_days,
        stale,
        capability_mismatch,
        missing_capabilities: missing,
    }
}

/// Where a hypothesis came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    /// Suggested by a stored finding: `memory:<finding id>`.
    Memory(String),
    /// Any other stated source, kept verbatim.
    Other(String),
}

impl Source {
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.trim().strip_prefix("memory:") {
            Some(id) if !id.trim().is_empty() => Self::Memory(id.trim().to_string()),
            _ => Self::Other(s.trim().to_string()),
        }
    }
}

/// A finding id as the store mints it.
#[must_use]
pub fn is_finding_id(id: &str) -> bool {
    id.starts_with("fnd_")
}

/// An experiment's evidence must be measurements, not memories of measurements.
///
/// # Errors
/// The offending ids: findings are cited as *where an idea came from*, never as the
/// evidence a new experiment rests on.
pub fn refuse_findings_as_evidence(evidence_ids: &[String]) -> Result<(), Vec<String>> {
    let bad: Vec<String> = evidence_ids.iter().filter(|i| is_finding_id(i)).cloned().collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad)
    }
}

/// What makes two hypotheses the same trial: the same finding as source, the same
/// instrument, and overlapping windows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepeatKey {
    pub finding: String,
    pub instrument: String,
    pub window: DataWindow,
}

/// Whether `new` repeats any of `earlier`; returns how many it repeats.
#[must_use]
pub fn flag_repeat(new: &RepeatKey, earlier: &[RepeatKey]) -> usize {
    earlier
        .iter()
        .filter(|e| e.finding == new.finding && e.instrument == new.instrument && e.window.overlaps(&new.window))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn age_staleness_and_capability_flags_are_present_on_every_annotation() {
        let a = annotate("2026-06-01", "2026-10-04", Some(90), false, &s(&["short_selling"]), Some(&s(&["long_only"])));
        assert_eq!(a.age_days, 125);
        assert!(a.stale);
        assert_eq!(a.capability_mismatch, Some(true));
        assert_eq!(a.missing_capabilities, s(&["short_selling"]));
        let fresh = annotate("2026-10-01", "2026-10-04", Some(90), false, &[], Some(&[]));
        assert!(!fresh.stale);
        assert_eq!(fresh.capability_mismatch, Some(false));
        // Not stated is unknown, never "fine".
        assert_eq!(annotate("2026-10-01", "2026-10-04", None, false, &s(&["x"]), None).capability_mismatch, None);
        // A kind that never expires on a timer is not stale by age, but a marked-stale row is.
        assert!(!annotate("2020-01-01", "2026-10-04", None, false, &[], None).stale);
        assert!(annotate("2026-10-01", "2026-10-04", None, true, &[], None).stale);
    }

    #[test]
    fn a_source_is_memory_only_with_a_finding_id_after_the_prefix() {
        assert_eq!(Source::parse("memory:fnd_abc"), Source::Memory("fnd_abc".into()));
        assert_eq!(Source::parse(" memory: fnd_abc "), Source::Memory("fnd_abc".into()));
        assert_eq!(Source::parse("memory:"), Source::Other("memory:".into()));
        assert_eq!(Source::parse("a paper"), Source::Other("a paper".into()));
    }

    #[test]
    fn an_experiment_citing_a_finding_as_evidence_is_refused_naming_it() {
        assert_eq!(refuse_findings_as_evidence(&s(&["exp_1", "art_2"])), Ok(()));
        assert_eq!(refuse_findings_as_evidence(&s(&["exp_1", "fnd_9"])), Err(s(&["fnd_9"])));
    }

    #[test]
    fn the_same_finding_on_the_same_instrument_and_window_is_a_repeated_trial() {
        let k = |f: &str, i: &str, a: &str, b: &str| RepeatKey {
            finding: f.into(),
            instrument: i.into(),
            window: DataWindow::new(a, b),
        };
        let earlier = vec![k("fnd_1", "BTC", "2026-01-01", "2026-03-01"), k("fnd_2", "BTC", "2026-01-01", "2026-03-01")];
        assert_eq!(flag_repeat(&k("fnd_1", "BTC", "2026-02-01", "2026-04-01"), &earlier), 1, "overlapping window");
        assert_eq!(flag_repeat(&k("fnd_1", "BTC", "2026-03-02", "2026-05-01"), &earlier), 0, "disjoint window");
        assert_eq!(flag_repeat(&k("fnd_1", "ETH", "2026-02-01", "2026-04-01"), &earlier), 0, "other instrument");
        assert_eq!(flag_repeat(&k("fnd_3", "BTC", "2026-02-01", "2026-04-01"), &earlier), 0, "other finding");
    }
}
