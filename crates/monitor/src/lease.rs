//! The lease: how "if the monitor dies, the platform stops" is enforced without
//! the monitor being in the process (COMP-007 §4).

use chrono::{DateTime, Duration, Utc};

/// How many times the monitor renews within one lease lifetime.
///
/// This is the number that makes COMP-007 §8.1 hold. A lease is good for exactly
/// **one** cadence, so a monitor that dies has its lease lapse within one cadence
/// and the runtime refuses — which is the acceptance criterion, stated in cadences.
/// But renewing only once per lifetime would mean a single scheduling hiccup on a
/// loaded box halts the book, and an operator who has watched that twice goes
/// looking for the flag that turns it off.
///
/// So the monitor renews three times per lifetime. A live monitor has to miss three
/// consecutive renewals to lose its lease; a dead one still loses it inside one
/// cadence. Lengthening the *lease* instead would have bought the same robustness
/// by making the detection slower, which is the half that matters.
pub const RENEWALS_PER_LEASE: i64 = 3;

/// The terms a monitor instance renews under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseTerms {
    pub lease_id: String,
    pub holder: String,
    pub cadence_ms: i64,
}

impl LeaseTerms {
    #[must_use]
    pub fn new(holder: impl Into<String>, cadence_ms: i64) -> Self {
        Self {
            lease_id: crate::LEASE_ID.to_owned(),
            holder: holder.into(),
            cadence_ms,
        }
    }

    /// When a renewal made at `now` expires: exactly one cadence later.
    #[must_use]
    pub fn expires_at(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        now + Duration::milliseconds(self.cadence_ms)
    }

    /// How often the monitor renews.
    #[must_use]
    pub fn renew_interval_ms(&self) -> i64 {
        (self.cadence_ms / RENEWALS_PER_LEASE).max(1)
    }
}

/// A lease row as read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    pub lease_id: String,
    pub holder: String,
    pub cadence_ms: i64,
    pub renewed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// What the order path concludes from the lease table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseStatus {
    /// A monitor renewed within its expiry. Orders may proceed.
    Valid,
    /// A row exists but has lapsed. The monitor is wedged, stopped, or gone.
    Expired,
    /// No row at all — no monitor has ever run against this database.
    Missing,
    /// The database could not be read.
    ///
    /// A distinct variant rather than a `Result`, because the *decision* is the
    /// same as `Missing` and collapsing them at the call site is how a `?` ends up
    /// propagating an error past a gate that should have closed. The order path
    /// must not be able to spell "I could not check, so carry on".
    Unreadable,
}

impl LeaseStatus {
    /// Whether new orders may proceed. Exactly one variant says yes.
    #[must_use]
    pub fn permits_orders(self) -> bool {
        matches!(self, Self::Valid)
    }

    /// Decide the status of what was read.
    #[must_use]
    pub fn of(lease: Option<&Lease>, now: DateTime<Utc>) -> Self {
        match lease {
            None => Self::Missing,
            Some(l) if l.expires_at > now => Self::Valid,
            Some(_) => Self::Expired,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("timestamp")
    }

    fn lease(expires: i64) -> Lease {
        Lease {
            lease_id: crate::LEASE_ID.to_owned(),
            holder: "m-1".to_owned(),
            cadence_ms: 60_000,
            renewed_at: at(0),
            expires_at: at(expires),
        }
    }

    #[test]
    fn a_fresh_lease_permits_orders() {
        assert_eq!(LeaseStatus::of(Some(&lease(120)), at(10)), LeaseStatus::Valid);
        assert!(LeaseStatus::of(Some(&lease(120)), at(10)).permits_orders());
    }

    #[test]
    fn a_lapsed_lease_refuses_rather_than_permits() {
        // The adversarial half of §4: the failure mode that matters is not a
        // monitor saying "halt", it is a monitor saying nothing at all.
        assert_eq!(LeaseStatus::of(Some(&lease(120)), at(121)), LeaseStatus::Expired);
        assert!(!LeaseStatus::of(Some(&lease(120)), at(121)).permits_orders());
    }

    #[test]
    fn expiry_is_exclusive_at_the_boundary() {
        // At exactly expires_at the lease is over. Treating the boundary as still
        // valid would mean a stopped monitor keeps the gate open for one more
        // instant on every clock that happens to land on it.
        assert_eq!(LeaseStatus::of(Some(&lease(120)), at(120)), LeaseStatus::Expired);
    }

    #[test]
    fn no_lease_at_all_refuses() {
        assert_eq!(LeaseStatus::of(None, at(10)), LeaseStatus::Missing);
        assert!(!LeaseStatus::of(None, at(10)).permits_orders());
    }

    #[test]
    fn an_unreadable_database_refuses_too() {
        // The one that is easy to get wrong: "I could not check" must not be a
        // pass, and there is no variant here that lets it be one.
        assert!(!LeaseStatus::Unreadable.permits_orders());
    }

    #[test]
    fn exactly_one_status_permits_orders() {
        let permitting: Vec<LeaseStatus> = [
            LeaseStatus::Valid,
            LeaseStatus::Expired,
            LeaseStatus::Missing,
            LeaseStatus::Unreadable,
        ]
        .into_iter()
        .filter(|s| s.permits_orders())
        .collect();
        assert_eq!(permitting, vec![LeaseStatus::Valid]);
    }

    #[test]
    fn a_dead_monitor_loses_its_lease_within_one_cadence() {
        // COMP-007 §8.1 is stated in cadences, so the lease lifetime is exactly
        // one. A two-cadence lease would have made the acceptance criterion false
        // while every individual piece of the design still looked right.
        let terms = LeaseTerms::new("m-1", 60_000);
        assert_eq!(terms.expires_at(at(0)), at(60));
    }

    #[test]
    fn a_live_monitor_has_three_chances_to_renew() {
        // And therefore survives a scheduling hiccup, which is what stops the
        // operator from going looking for the flag that turns this off.
        let terms = LeaseTerms::new("m-1", 60_000);
        assert_eq!(terms.renew_interval_ms(), 20_000);
        assert!(
            terms.renew_interval_ms() * RENEWALS_PER_LEASE <= terms.cadence_ms,
            "renewals must fit inside one lease lifetime"
        );
    }

    #[test]
    fn a_tiny_cadence_still_renews_at_least_once_per_millisecond_of_lease() {
        // Guards the integer division: a 1 ms cadence must not produce a renew
        // interval of zero, which would spin the monitor's loop at full speed.
        let terms = LeaseTerms::new("m-1", 1);
        assert_eq!(terms.renew_interval_ms(), 1);
    }
}
