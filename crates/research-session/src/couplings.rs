//! The couplings: the three things outside a session that stop it
//! (AGENT-014 §7 rows 8–10, §6.6; COMP-007 §6).
//!
//! A kill switch trips, a tier profile is withdrawn, or the platform itself
//! turns out to be broken. None of the three is something the session did, and
//! all three mean the same thing: **stop claiming**. What differs is the reason
//! the digest records and what has to become true again before it resumes.
//!
//! # Why absence pauses rather than proceeds
//!
//! Every input here is an *observation*, and an observation can fail to be
//! made. A driver that could not read the kill switch has not observed that it
//! is untripped — it has observed nothing — and reading nothing as "fine" is
//! precisely the failure mode COMP-007 exists to prevent. So [`Observed`] has a
//! third value and [`evaluate`] answers [`Verdict::CannotDecide`] for it, which
//! the driver treats as "do not claim" without appending a `paused` the human
//! would have to clear by hand.
//!
//! That is a deliberate asymmetry. A tripped switch is a **transition** the
//! session records and a human sees; an unreadable switch is a **failure of
//! this iteration**, and filing it as a pause would fill the digest with pauses
//! nobody caused and nobody can resolve.
//!
//! # Why the breaker is here and not in `harvest`
//!
//! §6.6's rule — two consecutive children failing on the same
//! `platform_defect` — is a statement about the session. [`crate::harvest`]
//! settles one run and says so in its own header: a function that can see one
//! run cannot decide a rule about two. [`DefectBreaker`] is the rule, and it
//! reads the session's finished runs in order.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::harvest::Ring;

/// Two consecutive children failing the same way opens the breaker (§6.6).
///
/// The same number as [`crate::agenda::MAX_ATTEMPTS`] and **not** the same
/// rule: that one expires one item after two goes, this one pauses the whole
/// session. They are written separately because they answer different
/// questions, and a single constant shared between them would make a change to
/// either silently change the other.
pub const DEFECT_TRIPS_AT: usize = 2;

/// A fact about the world the driver either measured or did not.
///
/// The third value is the point. See the module header.
/// Adjacently tagged, not internally tagged, and the difference is a landmine
/// rather than a style choice. `#[serde(tag = "kind")]` on a newtype variant
/// writes the payload's **fields** beside the tag, so a unit enum came out as
/// `{"kind":"is","armed":null}` — the measurement rendered as a key with a null
/// against it — and a payload that is not a map at all (an `f64`, a `String`)
/// fails at runtime with nothing failing at compile time. `content` puts the
/// value under its own key, which works for every `T`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Observed<T> {
    /// Read, and this is what it said.
    Is(T),
    /// The read did not happen. **Not** a value, and there is no `unwrap_or`
    /// here that would turn it into one.
    Unknown { why: String },
}

impl<T> Observed<T> {
    #[must_use]
    pub fn get(&self) -> Option<&T> {
        match self {
            Observed::Is(v) => Some(v),
            Observed::Unknown { .. } => None,
        }
    }

    #[must_use]
    pub fn unknown(why: &str) -> Self {
        Observed::Unknown {
            why: why.to_string(),
        }
    }
}

/// Where the platform's kill switch is (COMP-007 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KillSwitch {
    Armed,
    Tripped,
}

/// The health of the tier profile the session runs on (AGENT-009 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileHealth {
    /// Measured and passing: the only state a continuous session may claim in.
    Measured,
    /// Still present, but its scorecard has fallen.
    Degraded,
    /// Gone — the credential was removed, or the profile was retired.
    Withdrawn,
}

impl ProfileHealth {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ProfileHealth::Measured => "measured",
            ProfileHealth::Degraded => "degraded",
            ProfileHealth::Withdrawn => "withdrawn",
        }
    }

    /// §7 row 9: *"tier profile withdrawn **or degraded**"*. Both pause; only
    /// the word in the digest differs.
    #[must_use]
    pub fn pauses(self) -> bool {
        matches!(self, ProfileHealth::Degraded | ProfileHealth::Withdrawn)
    }
}

/// A child that ended on a platform defect rather than on a result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefectFailure {
    pub run: Uuid,
    pub item: Uuid,
    /// The tool the defect was filed against. `None` is a child that failed for
    /// a reason nobody attributed — which does **not** count toward the
    /// breaker and does break a streak, because a breaker opened on
    /// unattributed failures would name no tool to exclude.
    pub tool: Option<String>,
}

impl DefectFailure {
    #[must_use]
    pub fn on(run: Uuid, item: Uuid, tool: &str) -> Self {
        Self {
            run,
            item,
            tool: Some(tool.to_string()),
        }
    }

    #[must_use]
    pub fn unattributed(run: Uuid, item: Uuid) -> Self {
        Self {
            run,
            item,
            tool: None,
        }
    }
}

/// A child of the session, in the order it finished.
///
/// Carries only what §6.6 reads. A run that concluded — whatever its verdict —
/// is `Concluded`: a `reject` is a result, and a session that paused because
/// two ideas in a row were wrong would pause on doing its job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildEnd {
    Concluded,
    Defect(DefectFailure),
}

/// §6.6's rule: two consecutive children failing on the same platform defect.
///
/// *Consecutive* is the spec's own word and it is load-bearing. A defect in one
/// tool, a successful run, then the same defect again is a flaky tool, not a
/// broken platform, and pausing a session over it spends a human's attention on
/// something that fixed itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefectBreaker {
    /// The tool the streak is on.
    pub tool: String,
    /// Every run in the streak, in order, so the pause names them.
    pub runs: Vec<Uuid>,
}

impl DefectBreaker {
    /// Whether the session's children have opened a breaker.
    ///
    /// Reads the tail: the streak has to be the *most recent* runs, because a
    /// streak in a session's distant past was either resolved or is still
    /// there, and if it is still there the tail will say so.
    #[must_use]
    pub fn from_children(children: &[ChildEnd]) -> Option<Self> {
        let mut tool: Option<&str> = None;
        let mut runs: Vec<Uuid> = Vec::new();
        for end in children.iter().rev() {
            match end {
                ChildEnd::Defect(f) => match (&f.tool, tool) {
                    // An unattributed failure names no tool to exclude, so it
                    // cannot extend a streak and it ends one.
                    (None, _) => break,
                    (Some(t), None) => {
                        tool = Some(t);
                        runs.push(f.run);
                    }
                    (Some(t), Some(seen)) if t == seen => runs.push(f.run),
                    (Some(_), Some(_)) => break,
                },
                ChildEnd::Concluded => break,
            }
        }
        if runs.len() >= DEFECT_TRIPS_AT {
            runs.reverse();
            return Some(Self {
                tool: tool.unwrap_or_default().to_string(),
                runs,
            });
        }
        None
    }

    /// The `platform_defect` memory entry this breaker asks the driver to file
    /// (§7 row 10).
    ///
    /// The *sentence*, not the row: `harness::memory::Entry` is the row, it
    /// wants a provenance this module does not have, and building one here
    /// would be a second writer of a kind with one constructor.
    #[must_use]
    pub fn entry_claim(&self) -> String {
        format!(
            "`{}` failed {} consecutive research runs in one session: {}",
            self.tool,
            self.runs.len(),
            self.runs
                .iter()
                .map(|r| r.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// Why a session is paused by something outside it (§7 rows 8–10).
///
/// The word is what `session_paused`'s payload carries and what the digest
/// prints; the rest is what makes the line resolve on click.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PauseReason {
    KillSwitch,
    Tier {
        profile: String,
        health: String,
    },
    PlatformDefect {
        tool: String,
        runs: Vec<Uuid>,
    },
}

impl PauseReason {
    /// The word `session_paused` records.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            PauseReason::KillSwitch => "kill_switch",
            PauseReason::Tier { .. } => "tier",
            PauseReason::PlatformDefect { .. } => "platform_defect",
        }
    }

    /// The pause words, in the order [`evaluate`] reports them.
    pub const WORDS: &'static [&'static str] = &["kill_switch", "tier", "platform_defect"];

    /// Whether live children drain at the boundary, or are left to finish
    /// (AGENT-014 §7, rows 8-10).
    ///
    /// The three rows do **not** say the same thing and the difference is not
    /// cosmetic. Row 8: *"live children finish the step and suspend"*. Row 10:
    /// *"budget spent on a broken platform is not research"*, which is a
    /// sentence about stopping the spend now. Row 9 is the odd one: *"children
    /// in flight **finish**"* — a degraded profile is still a profile, the work
    /// already paid for is worth having, and suspending it would throw away a
    /// run that was going to produce something.
    ///
    /// An exhaustive match rather than a default arm, so a fourth coupling has
    /// to state which it is instead of inheriting whichever was written last.
    #[must_use]
    pub fn drains_live_children(&self) -> bool {
        match self {
            PauseReason::KillSwitch | PauseReason::PlatformDefect { .. } => true,
            PauseReason::Tier { .. } => false,
        }
    }

    /// One sentence for the digest.
    #[must_use]
    pub fn sentence(&self) -> String {
        match self {
            PauseReason::KillSwitch => {
                "the platform kill switch is tripped; nothing claims and no ring-2 proposal can \
                 be created until it is reset"
                    .to_string()
            }
            PauseReason::Tier { profile, health } => format!(
                "the profile `{profile}` is {health}; continuous research runs on a measured \
                 frontier profile and this one is not one"
            ),
            PauseReason::PlatformDefect { tool, runs } => format!(
                "`{tool}` failed {} consecutive runs; budget spent on a broken platform is not \
                 research",
                runs.len()
            ),
        }
    }
}

/// What the driver measured this iteration about the world outside the session.
#[derive(Debug, Clone, PartialEq)]
pub struct Signals {
    pub kill_switch: Observed<KillSwitch>,
    /// The profile the session runs on, and its health.
    pub profile: String,
    pub health: Observed<ProfileHealth>,
    /// The session's children, oldest first.
    pub children: Vec<ChildEnd>,
}

impl Signals {
    /// The healthy case, for tests and for a session whose world is fine.
    #[must_use]
    pub fn nominal(profile: &str) -> Self {
        Self {
            kill_switch: Observed::Is(KillSwitch::Armed),
            profile: profile.to_string(),
            health: Observed::Is(ProfileHealth::Measured),
            children: Vec::new(),
        }
    }
}

/// What the couplings say the session may do now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing outside the session is stopping it.
    Clear,
    /// One or more couplings are pausing it. **Every** reason is carried, not
    /// just the first: a session resumed on a kill-switch reset while its
    /// profile is still withdrawn is a session that resumed into the second
    /// thing that was wrong with it.
    Paused(Vec<PauseReason>),
    /// A reading did not happen. The session claims nothing this iteration and
    /// **no transition is appended** — see the module header.
    CannotDecide { missing: Vec<String> },
}

impl Verdict {
    /// Whether the driver may claim an item.
    #[must_use]
    pub fn may_claim(&self) -> bool {
        matches!(self, Verdict::Clear)
    }

    /// Whether this verdict is something the log should record as a transition.
    ///
    /// `CannotDecide` is not: it is a failed iteration, not a state the session
    /// is in, and appending it would need a human to clear a pause nobody
    /// caused.
    #[must_use]
    pub fn is_a_transition(&self) -> bool {
        matches!(self, Verdict::Paused(_))
    }

    /// The primary reason, which is the one the `session_paused` event's word
    /// carries.
    #[must_use]
    pub fn primary(&self) -> Option<&PauseReason> {
        match self {
            Verdict::Paused(rs) => rs.first(),
            _ => None,
        }
    }
}

/// Read the couplings (AGENT-014 §7 rows 8–10).
///
/// The order is severity, and it is the order [`PauseReason::WORDS`] is written
/// in: the kill switch is the one about the market, the tier is the one about
/// whether the session can think, and the defect is the one about whether the
/// platform works. A session with all three has all three in the digest, and
/// the first is the word on the event.
#[must_use]
pub fn evaluate(signals: &Signals) -> Verdict {
    let mut missing = Vec::new();
    let mut reasons = Vec::new();

    match signals.kill_switch.get() {
        Some(KillSwitch::Tripped) => reasons.push(PauseReason::KillSwitch),
        Some(KillSwitch::Armed) => {}
        None => missing.push(match &signals.kill_switch {
            Observed::Unknown { why } => format!("the kill switch: {why}"),
            Observed::Is(_) => unreachable!("`get` answered None"),
        }),
    }

    match signals.health.get() {
        Some(h) if h.pauses() => reasons.push(PauseReason::Tier {
            profile: signals.profile.clone(),
            health: h.as_str().to_string(),
        }),
        Some(_) => {}
        None => missing.push(match &signals.health {
            Observed::Unknown { why } => {
                format!("the health of profile `{}`: {why}", signals.profile)
            }
            Observed::Is(_) => unreachable!("`get` answered None"),
        }),
    }

    // The breaker reads the session's own log, which the driver has by
    // construction — there is no read here that could fail, so no third value.
    if let Some(b) = DefectBreaker::from_children(&signals.children) {
        reasons.push(PauseReason::PlatformDefect {
            tool: b.tool,
            runs: b.runs,
        });
    }

    // Unreadable beats paused. A session whose kill switch could not be read
    // and whose profile is degraded must not be recorded as `paused{tier}`:
    // the tier would be cleared, the session would resume, and nobody would
    // ever have found out about the switch.
    if !missing.is_empty() {
        return Verdict::CannotDecide { missing };
    }
    if reasons.is_empty() {
        Verdict::Clear
    } else {
        Verdict::Paused(reasons)
    }
}

/// Whether a proposal into this ring may be created right now
/// (AGENT-013 §6.8, AGENT-014 §7 row 8).
///
/// While the kill switch is tripped, nothing may propose into ring 2 — a
/// `deploy_paper` is a request to put something in front of a market the
/// platform has just halted. Ring 1 is artifacts and is unaffected.
///
/// Returns the refusal sentence rather than a bool, because a refusal a caller
/// renders itself is a refusal that reads differently in each of the three
/// places it is rendered.
#[must_use]
pub fn refuses_ring(signals: &Signals, ring: Ring) -> Option<String> {
    // `Unknown` refuses too. A proposal created during an unreadable kill
    // switch is the one case where guessing costs more than waiting.
    let tripped = !matches!(signals.kill_switch, Observed::Is(KillSwitch::Armed));
    if tripped && ring == Ring::Paper {
        return Some(
            "the kill switch is not confirmed armed; a ring-2 proposal asks a human to put \
             something in front of a market the platform has halted, so it is not created while \
             that is true (AGENT-013 §6.8)"
                .to_string(),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(n: u128) -> Uuid {
        Uuid::from_u128(0x7000_0000 + n)
    }

    fn item(n: u128) -> Uuid {
        Uuid::from_u128(0x8000_0000 + n)
    }

    #[test]
    fn a_nominal_world_lets_the_session_claim() {
        assert_eq!(evaluate(&Signals::nominal("opus")), Verdict::Clear);
        assert!(evaluate(&Signals::nominal("opus")).may_claim());
    }

    #[test]
    fn a_tripped_switch_pauses_and_names_itself() {
        let mut s = Signals::nominal("opus");
        s.kill_switch = Observed::Is(KillSwitch::Tripped);
        let v = evaluate(&s);
        assert!(!v.may_claim());
        assert_eq!(v.primary(), Some(&PauseReason::KillSwitch));
        assert_eq!(v.primary().unwrap().word(), "kill_switch");
    }

    #[test]
    fn a_reset_switch_resumes_without_anything_else_having_to_happen() {
        // §7 row 8: "automatically on reset".
        let mut s = Signals::nominal("opus");
        s.kill_switch = Observed::Is(KillSwitch::Tripped);
        assert!(!evaluate(&s).may_claim());
        s.kill_switch = Observed::Is(KillSwitch::Armed);
        assert!(evaluate(&s).may_claim());
    }

    #[test]
    fn both_a_degraded_and_a_withdrawn_profile_pause() {
        for h in [ProfileHealth::Degraded, ProfileHealth::Withdrawn] {
            let mut s = Signals::nominal("opus");
            s.health = Observed::Is(h);
            let v = evaluate(&s);
            assert_eq!(
                v.primary(),
                Some(&PauseReason::Tier {
                    profile: "opus".into(),
                    health: h.as_str().into(),
                }),
                "{h:?}"
            );
        }
    }

    #[test]
    fn an_unreadable_switch_is_not_an_armed_one() {
        // The whole rule this crate is built on, at the one place where reading
        // absence as "fine" would let a session keep spending through a halt.
        let mut s = Signals::nominal("opus");
        s.kill_switch = Observed::unknown("the monitor's table did not answer");
        let v = evaluate(&s);
        assert!(!v.may_claim());
        assert!(matches!(v, Verdict::CannotDecide { .. }), "{v:?}");
        assert!(
            !v.is_a_transition(),
            "a failed read is not a state a human has to clear"
        );
    }

    #[test]
    fn an_unreadable_signal_beats_a_pause_so_the_pause_does_not_hide_it() {
        let mut s = Signals::nominal("opus");
        s.kill_switch = Observed::unknown("timed out");
        s.health = Observed::Is(ProfileHealth::Degraded);
        let v = evaluate(&s);
        // If this returned `Paused{tier}`, restoring the profile would resume
        // the session and nobody would ever learn the switch was unreadable.
        assert!(matches!(v, Verdict::CannotDecide { .. }), "{v:?}");
        let Verdict::CannotDecide { missing } = v else {
            unreachable!()
        };
        assert!(missing[0].contains("kill switch"), "{missing:?}");
    }

    #[test]
    fn every_reason_is_carried_not_only_the_first() {
        let mut s = Signals::nominal("opus");
        s.kill_switch = Observed::Is(KillSwitch::Tripped);
        s.health = Observed::Is(ProfileHealth::Withdrawn);
        s.children = vec![
            ChildEnd::Defect(DefectFailure::on(run(1), item(1), "backtest")),
            ChildEnd::Defect(DefectFailure::on(run(2), item(2), "backtest")),
        ];
        let Verdict::Paused(reasons) = evaluate(&s) else {
            panic!("three couplings are pausing this session");
        };
        assert_eq!(reasons.len(), 3);
        let words: Vec<&str> = reasons.iter().map(PauseReason::word).collect();
        assert_eq!(words, PauseReason::WORDS);
    }

    #[test]
    fn two_consecutive_defects_on_one_tool_open_the_breaker() {
        let children = vec![
            ChildEnd::Concluded,
            ChildEnd::Defect(DefectFailure::on(run(1), item(1), "backtest")),
            ChildEnd::Defect(DefectFailure::on(run(2), item(2), "backtest")),
        ];
        let b = DefectBreaker::from_children(&children).expect("two in a row on one tool");
        assert_eq!(b.tool, "backtest");
        assert_eq!(b.runs, vec![run(1), run(2)], "oldest first, both named");
    }

    #[test]
    fn a_successful_run_between_two_defects_is_a_flaky_tool_not_a_broken_platform() {
        // §6.6's word is "consecutive", and it is load-bearing: without it, any
        // session long enough eventually pauses on a tool that works.
        let children = vec![
            ChildEnd::Defect(DefectFailure::on(run(1), item(1), "backtest")),
            ChildEnd::Concluded,
            ChildEnd::Defect(DefectFailure::on(run(2), item(2), "backtest")),
        ];
        assert_eq!(DefectBreaker::from_children(&children), None);
    }

    #[test]
    fn two_defects_on_two_different_tools_are_not_a_streak() {
        let children = vec![
            ChildEnd::Defect(DefectFailure::on(run(1), item(1), "backtest")),
            ChildEnd::Defect(DefectFailure::on(run(2), item(2), "sweep")),
        ];
        assert_eq!(DefectBreaker::from_children(&children), None);
    }

    #[test]
    fn an_unattributed_failure_cannot_open_a_breaker_that_names_no_tool() {
        // §7 row 10's resume is "the user resumes with the tool excluded". A
        // pause naming no tool offers nothing to exclude.
        let children = vec![
            ChildEnd::Defect(DefectFailure::unattributed(run(1), item(1))),
            ChildEnd::Defect(DefectFailure::unattributed(run(2), item(2))),
        ];
        assert_eq!(DefectBreaker::from_children(&children), None);
    }

    #[test]
    fn an_unattributed_failure_between_two_defects_breaks_the_streak() {
        let children = vec![
            ChildEnd::Defect(DefectFailure::on(run(1), item(1), "backtest")),
            ChildEnd::Defect(DefectFailure::unattributed(run(9), item(9))),
            ChildEnd::Defect(DefectFailure::on(run(2), item(2), "backtest")),
        ];
        assert_eq!(DefectBreaker::from_children(&children), None);
    }

    #[test]
    fn a_longer_streak_names_every_run_in_it() {
        let children = vec![
            ChildEnd::Defect(DefectFailure::on(run(1), item(1), "backtest")),
            ChildEnd::Defect(DefectFailure::on(run(2), item(2), "backtest")),
            ChildEnd::Defect(DefectFailure::on(run(3), item(3), "backtest")),
        ];
        let b = DefectBreaker::from_children(&children).expect("three in a row");
        assert_eq!(b.runs.len(), 3);
        assert!(b.entry_claim().contains("3 consecutive"));
        assert!(b.entry_claim().contains("backtest"));
    }

    #[test]
    fn a_session_whose_ideas_were_merely_wrong_does_not_pause() {
        // A `reject` is a result. A session pausing on two of them would pause
        // on doing its job.
        let children = vec![ChildEnd::Concluded, ChildEnd::Concluded];
        assert_eq!(DefectBreaker::from_children(&children), None);
        let mut s = Signals::nominal("opus");
        s.children = children;
        assert!(evaluate(&s).may_claim());
    }

    #[test]
    fn no_ring_two_proposal_is_created_while_the_switch_is_tripped() {
        let mut s = Signals::nominal("opus");
        s.kill_switch = Observed::Is(KillSwitch::Tripped);
        assert!(refuses_ring(&s, Ring::Paper).is_some());
        assert!(
            refuses_ring(&s, Ring::Artifacts).is_none(),
            "ring 1 is artifacts and touches no market"
        );
    }

    #[test]
    fn an_unreadable_switch_also_refuses_a_ring_two_proposal() {
        let mut s = Signals::nominal("opus");
        s.kill_switch = Observed::unknown("timed out");
        assert!(
            refuses_ring(&s, Ring::Paper).is_some(),
            "the one case where guessing costs more than waiting"
        );
    }

    #[test]
    fn an_armed_switch_refuses_nothing() {
        let s = Signals::nominal("opus");
        assert!(refuses_ring(&s, Ring::Paper).is_none());
        assert!(refuses_ring(&s, Ring::Artifacts).is_none());
    }

    #[test]
    fn every_pause_reason_has_a_sentence_that_says_what_to_do() {
        let reasons = [
            PauseReason::KillSwitch,
            PauseReason::Tier {
                profile: "opus".into(),
                health: "withdrawn".into(),
            },
            PauseReason::PlatformDefect {
                tool: "backtest".into(),
                runs: vec![run(1), run(2)],
            },
        ];
        for r in &reasons {
            let s = r.sentence();
            assert!(s.len() > 40, "a pause a human cannot act on: {s}");
        }
        assert_eq!(
            reasons.iter().map(PauseReason::word).collect::<Vec<_>>(),
            PauseReason::WORDS
        );
    }

    #[test]
    fn the_three_rows_do_not_all_drain_and_the_one_that_does_not_is_the_tier() {
        // §7 row 9 says children in flight **finish**; rows 8 and 10 say they
        // suspend. A single "pause means drain" rule would throw away a run a
        // degraded profile was about to complete.
        assert!(PauseReason::KillSwitch.drains_live_children());
        assert!(PauseReason::PlatformDefect {
            tool: "backtest".into(),
            runs: vec![run(1)],
        }
        .drains_live_children());
        assert!(!PauseReason::Tier {
            profile: "opus".into(),
            health: "degraded".into(),
        }
        .drains_live_children());
    }

    #[test]
    fn an_observation_survives_a_round_trip_whatever_it_carries() {
        // The tagging this enum used to have wrote a unit payload out as a
        // *key* — `{"kind":"is","armed":null}` — and would have failed outright
        // on an `Observed<f64>`, which is the shape `execute` needs. Both
        // directions are asserted here because the wire form is the thing a
        // reader of a stored event sees, and it had never been looked at.
        let a = Observed::Is(KillSwitch::Tripped);
        let j = serde_json::to_string(&a).expect("a unit payload");
        assert_eq!(j, r#"{"kind":"is","value":"tripped"}"#);
        assert_eq!(
            serde_json::from_str::<Observed<KillSwitch>>(&j).expect("back again"),
            a
        );

        let n: Observed<i64> = Observed::Is(7);
        let j = serde_json::to_string(&n).expect("a primitive payload, which used to fail");
        assert_eq!(j, r#"{"kind":"is","value":7}"#);

        let u: Observed<KillSwitch> = Observed::unknown("monitor.events did not answer");
        let j = serde_json::to_string(&u).expect("an absence");
        assert_eq!(
            serde_json::from_str::<Observed<KillSwitch>>(&j).expect("back again"),
            u,
            "an absence that deserialised as a value would be the whole bug"
        );
    }
}
