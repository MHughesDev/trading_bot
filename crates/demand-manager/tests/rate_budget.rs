//! P2-T02 acceptance tests: admit up to cap; next admission denied; release frees budget.

use demand_manager::{RateBudget, VenueBudget};
use domain::SupportedVenue;

fn budget_with_cap(cap: u32) -> RateBudget {
    RateBudget::new([(SupportedVenue::Kraken, VenueBudget::new(cap))])
}

#[test]
fn admit_up_to_cap_succeeds() {
    let budget = budget_with_cap(3);
    for _ in 0..3 {
        assert!(budget.try_admit(SupportedVenue::Kraken).is_ok());
    }
    assert_eq!(budget.active(SupportedVenue::Kraken), 3);
}

#[test]
fn admission_beyond_cap_is_denied() {
    let budget = budget_with_cap(2);
    budget.try_admit(SupportedVenue::Kraken).unwrap();
    budget.try_admit(SupportedVenue::Kraken).unwrap();

    let err = budget.try_admit(SupportedVenue::Kraken).unwrap_err();
    assert_eq!(err.active, 2);
    assert_eq!(err.max, 2);
    assert!(err.to_string().contains("kraken"));
}

#[test]
fn release_frees_one_slot() {
    let budget = budget_with_cap(2);
    budget.try_admit(SupportedVenue::Kraken).unwrap();
    budget.try_admit(SupportedVenue::Kraken).unwrap();

    // At cap — next admit denied.
    assert!(budget.try_admit(SupportedVenue::Kraken).is_err());

    // Release one slot.
    budget.release(SupportedVenue::Kraken);
    assert_eq!(budget.active(SupportedVenue::Kraken), 1);

    // Now admit succeeds again.
    assert!(budget.try_admit(SupportedVenue::Kraken).is_ok());
}

#[test]
fn release_below_zero_is_safe() {
    let budget = budget_with_cap(5);
    // Release without prior admit — should not panic.
    budget.release(SupportedVenue::Kraken);
    assert_eq!(budget.active(SupportedVenue::Kraken), 0);
}

#[test]
fn different_venues_are_independent() {
    let budget = RateBudget::new([
        (SupportedVenue::Kraken, VenueBudget::new(1)),
        (SupportedVenue::Alpaca, VenueBudget::new(1)),
    ]);

    budget.try_admit(SupportedVenue::Kraken).unwrap();
    // Kraken is at cap.
    assert!(budget.try_admit(SupportedVenue::Kraken).is_err());
    // Alpaca is independent.
    assert!(budget.try_admit(SupportedVenue::Alpaca).is_ok());
}
