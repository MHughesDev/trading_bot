//! The human controls: reset, resume, and the rules that make them human
//! (COMP-007 §5, §8.7).
//!
//! Reset is the one action in this system that is always a person. Everything else
//! the monitor does is either reversible (scale-down) or is the safe direction
//! (halt); a reset is irreversible in the only sense that matters, because the
//! orders that go out afterwards cannot be recalled.
//!
//! Three requirements, and each of them exists because of a specific way operators
//! actually behave under pressure:
//!
//! - **Typed confirmation of the scope name.** A yes/no dialog at 3 a.m. is a
//!   reflex. Typing `venue:kraken` is not, and it is the difference between
//!   resuming the venue you meant and resuming the whole platform.
//! - **A reason.** Not for the audit — for the operator, five minutes later, when
//!   they are trying to remember whether they already looked at this.
//! - **A rate limit, one per five minutes per scope.** A flapping fault can
//!   otherwise be reset into: trip, reset, trip, reset, each one individually
//!   defensible, and forty minutes later nobody has looked at why.

use chrono::{DateTime, Duration, Utc};
use risk::Scope;

/// One reset per this many seconds, per scope (COMP-007 §5).
pub const RESET_RATE_LIMIT_SECS: i64 = 300;

/// A request to release a halt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResetRequest {
    pub scope: Scope,
    /// What the human typed. Must equal the scope's own string form.
    pub typed_confirmation: String,
    pub reason: String,
    /// `human:<user_id>`.
    pub actor: String,
    /// For a **global** reset only: whether every venue currently reconciles
    /// clean. The rule lives here so that the API and the monitor cannot disagree
    /// about it.
    ///
    /// **Overwritten by [`MonitorStore::reset`][r]** from
    /// `monitor.venue_reconciliation` (0165) before validation: a precondition
    /// the caller asserts is not a precondition. A caller's value is kept only
    /// by code that calls [`Self::validate`] directly, which is tests.
    ///
    /// [r]: crate::store::MonitorStore::reset
    pub all_venues_reconciled: bool,
}

/// Why a reset was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResetRefusal {
    /// The typed text did not match the scope.
    ConfirmationMismatch { expected: String, typed: String },
    /// No reason given.
    ReasonMissing,
    /// Something that is not a person tried to reset the switch.
    NotHuman { actor: String },
    /// A global reset while a venue still fails reconciliation.
    VenuesNotReconciled,
    /// A venue reset while that venue's last reconciliation did not agree
    /// (COMP-007 §8.3: "resume is refused until reconciliation passes").
    VenueNotReconciled { venue: String },
    /// Another reset of this scope less than five minutes ago.
    RateLimited { retry_after_secs: i64 },
}

impl std::fmt::Display for ResetRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfirmationMismatch { expected, typed } => write!(
                f,
                "type {expected:?} to confirm this reset; received {typed:?}"
            ),
            Self::ReasonMissing => f.write_str("a reset requires a reason"),
            Self::NotHuman { actor } => write!(f, "{actor} may not reset the kill switch"),
            Self::VenuesNotReconciled => {
                f.write_str("a global reset requires every venue to reconcile clean first")
            }
            Self::VenueNotReconciled { venue } => write!(
                f,
                "venue {venue} last failed reconciliation; it resumes only after a                  reconciliation of it agrees with our book"
            ),
            Self::RateLimited { retry_after_secs } => write!(
                f,
                "this scope was reset less than {RESET_RATE_LIMIT_SECS}s ago; \
                 retry in {retry_after_secs}s"
            ),
        }
    }
}

impl std::error::Error for ResetRefusal {}

impl ResetRequest {
    /// The venue half of §8.3, separate from [`Self::validate`] because its input
    /// is a fact about one venue that only the store can read.
    ///
    /// `venue_reconciled` is `monitor.venue_reconciled(venue)`: never checked, or
    /// last check agreed. A scope that is not a venue is not held to it.
    ///
    /// # Errors
    /// [`ResetRefusal::VenueNotReconciled`] for a venue whose last check did not
    /// agree.
    pub fn validate_venue(&self, venue_reconciled: bool) -> Result<(), ResetRefusal> {
        match &self.scope {
            Scope::Venue(v) if !venue_reconciled => Err(ResetRefusal::VenueNotReconciled {
                venue: v.clone(),
            }),
            _ => Ok(()),
        }
    }

    /// The whole rule set. Nothing is written until this returns `Ok`.
    pub fn validate(
        &self,
        last_reset_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<(), ResetRefusal> {
        // Checked first, and by shape rather than by scope list: an actor that is
        // not a named human has no business here whatever else is true, and an
        // `agent:` prefix is only the case we already know about.
        if !self.actor.starts_with("human:") || self.actor.len() <= "human:".len() {
            return Err(ResetRefusal::NotHuman {
                actor: self.actor.clone(),
            });
        }
        if self.reason.trim().is_empty() {
            return Err(ResetRefusal::ReasonMissing);
        }
        let expected = self.scope.to_string();
        if self.typed_confirmation != expected {
            return Err(ResetRefusal::ConfirmationMismatch {
                expected,
                typed: self.typed_confirmation.clone(),
            });
        }
        if self.scope == Scope::Global && !self.all_venues_reconciled {
            return Err(ResetRefusal::VenuesNotReconciled);
        }
        if let Some(last) = last_reset_at {
            let elapsed = now - last;
            let limit = Duration::seconds(RESET_RATE_LIMIT_SECS);
            if elapsed < limit {
                return Err(ResetRefusal::RateLimited {
                    retry_after_secs: (limit - elapsed).num_seconds().max(1),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("timestamp")
    }

    fn global() -> ResetRequest {
        ResetRequest {
            scope: Scope::Global,
            typed_confirmation: "global".to_owned(),
            reason: "collector restarted; feeds are current again".to_owned(),
            actor: "human:mason".to_owned(),
            all_venues_reconciled: true,
        }
    }

    #[test]
    fn a_typed_reasoned_reconciled_global_reset_is_allowed() {
        assert!(global().validate(None, at(0)).is_ok());
    }

    #[test]
    fn a_global_reset_without_the_typed_scope_is_refused() {
        // COMP-007 §8.7, first half.
        let mut r = global();
        r.typed_confirmation = "yes".to_owned();
        assert!(matches!(
            r.validate(None, at(0)),
            Err(ResetRefusal::ConfirmationMismatch { .. })
        ));
    }

    #[test]
    fn confirming_the_wrong_scope_is_refused() {
        // The failure this actually catches: an operator with two incidents open
        // types the venue they were looking at a moment ago and resumes the book.
        let mut r = global();
        r.typed_confirmation = "venue:kraken".to_owned();
        assert!(r.validate(None, at(0)).is_err());
    }

    #[test]
    fn a_reset_without_a_reason_is_refused() {
        let mut r = global();
        r.reason = "   ".to_owned();
        assert_eq!(r.validate(None, at(0)), Err(ResetRefusal::ReasonMissing));
    }

    #[test]
    fn a_second_reset_within_five_minutes_is_refused() {
        // COMP-007 §8.7, second half. A flapping fault must not be reset into.
        let r = global();
        let err = r.validate(Some(at(0)), at(299)).expect_err("too soon");
        match err {
            ResetRefusal::RateLimited { retry_after_secs } => assert_eq!(retry_after_secs, 1),
            other => panic!("wrong refusal: {other:?}"),
        }
        assert!(r.validate(Some(at(0)), at(300)).is_ok());
    }

    #[test]
    fn the_rate_limit_is_per_scope_and_not_global() {
        // Two different faults are two different incidents; making one operator's
        // venue reset block another venue's would push people toward resetting
        // `global` instead, which is the opposite of what this is for.
        let venue = ResetRequest {
            scope: Scope::venue("kraken"),
            typed_confirmation: "venue:kraken".to_owned(),
            reason: "reconciled clean".to_owned(),
            actor: "human:mason".to_owned(),
            all_venues_reconciled: false,
        };
        // A global reset five seconds ago says nothing about this scope, because
        // the caller looks up `last_reset_at` for *this* scope.
        assert!(venue.validate(None, at(5)).is_ok());
    }

    #[test]
    fn a_global_reset_is_refused_while_a_venue_still_fails_reconciliation() {
        let mut r = global();
        r.all_venues_reconciled = false;
        assert_eq!(
            r.validate(None, at(0)),
            Err(ResetRefusal::VenuesNotReconciled)
        );
    }

    #[test]
    fn a_venue_reset_does_not_require_every_other_venue_to_be_clean() {
        let r = ResetRequest {
            scope: Scope::venue("kraken"),
            typed_confirmation: "venue:kraken".to_owned(),
            reason: "positions match after the manual fix".to_owned(),
            actor: "human:mason".to_owned(),
            all_venues_reconciled: false,
        };
        assert!(r.validate(None, at(0)).is_ok());
    }

    #[test]
    fn a_venue_whose_last_reconciliation_disagreed_cannot_be_resumed() {
        let r = ResetRequest {
            scope: Scope::venue("kraken"),
            typed_confirmation: "venue:kraken".to_owned(),
            reason: "looks fine to me".to_owned(),
            actor: "human:mason".to_owned(),
            all_venues_reconciled: true,
        };
        assert_eq!(
            r.validate_venue(false),
            Err(ResetRefusal::VenueNotReconciled {
                venue: "kraken".to_owned()
            })
        );
        assert!(r.validate_venue(true).is_ok());
        // Global and strategy scopes are not venue resumes.
        assert!(global().validate_venue(false).is_ok());
    }

    #[test]
    fn nothing_that_is_not_a_named_human_can_reset() {
        // Asserted over the shapes that actually appear in `tripped_by`, not just
        // over `agent:`: 'monitor' trips the switch and must not be able to
        // release it either, or the fail-closed design has a self-clearing hole.
        for actor in [
            "agent:run-9",
            "monitor",
            "drill",
            "",
            "human:",
            "service:deployer",
        ] {
            let mut r = global();
            r.actor = actor.to_owned();
            assert!(
                matches!(r.validate(None, at(0)), Err(ResetRefusal::NotHuman { .. })),
                "{actor:?} must not be able to reset"
            );
        }
    }
}
