//! The capital ramp (SPEC §12.3 Gate 16, checklist 2.14, ADR-P2-14).
//!
//! "The registry is the only path to capital." That sentence is either a slogan
//! or a type, and here it is a type.
//!
//! [`AllowedFraction`] is the fraction of the intended size a strategy may
//! trade. It takes five values and nothing else, it has no public constructor,
//! and the only way to raise one is [`AllowedFraction::raise`], which takes a
//! [`RampAuthority`] — a token that [`authorise`] mints only when every gate in
//! a **capital-authorising** profile has passed. `paper_v1` does not authorise
//! capital (`gate_profile.authorises_capital = FALSE`), so a `paper_v1` pass
//! cannot produce the token, and therefore cannot produce a fraction above zero.
//! Not "should not" — cannot construct one (AT-65).
//!
//! Lowering is the opposite: [`AllowedFraction::step_down`] needs no authority
//! at all. Making a strategy smaller is never the dangerous direction, and a
//! reduction that required a committee is a reduction that happens late.

use serde::{Deserialize, Serialize};

/// The five rungs. §12.3 fixes the ladder; there is no sixth value and no
/// arbitrary fraction, because "we're running it at 37 %" is a number somebody
/// picked rather than a rung somebody was promoted to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllowedFraction {
    /// Paper only. The floor, and the value every strategy starts at.
    Zero,
    Ten,
    TwentyFive,
    Fifty,
    Full,
}

use AllowedFraction::{Fifty, Full, Ten, TwentyFive, Zero};

impl AllowedFraction {
    /// The ladder, in order.
    pub const LADDER: [Self; 5] = [Zero, Ten, TwentyFive, Fifty, Full];

    #[must_use]
    pub fn value(self) -> f64 {
        match self {
            Zero => 0.0,
            Ten => 0.10,
            TwentyFive => 0.25,
            Fifty => 0.50,
            Full => 1.0,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Zero => "zero",
            Ten => "ten",
            TwentyFive => "twenty_five",
            Fifty => "fifty",
            Full => "full",
        }
    }

    /// Exact parse of a stored rung.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::LADDER.into_iter().find(|f| f.as_str() == code)
    }

    /// The rung for a stored numeric fraction. Exact: a value that is not a rung
    /// is not rounded to the nearest one, because rounding up is the direction
    /// that costs money.
    #[must_use]
    pub fn from_value(v: f64) -> Option<Self> {
        Self::LADDER.into_iter().find(|f| (f.value() - v).abs() < 1e-9)
    }

    fn index(self) -> usize {
        match self {
            Zero => 0,
            Ten => 1,
            TwentyFive => 2,
            Fifty => 3,
            Full => 4,
        }
    }

    /// Raise by exactly one rung, with authority.
    ///
    /// One rung, because a ramp that can jump from paper to full size is not a
    /// ramp. Already at the top: stays there.
    #[must_use]
    pub fn raise(self, _authority: &RampAuthority) -> Self {
        Self::LADDER[(self.index() + 1).min(Self::LADDER.len() - 1)]
    }

    /// Lower by one rung. No authority required, ever.
    #[must_use]
    pub fn step_down(self) -> Self {
        Self::LADDER[self.index().saturating_sub(1)]
    }

    /// All the way down, immediately. The response to a kill criterion firing.
    #[must_use]
    pub fn halt() -> Self {
        Zero
    }
}

impl Default for AllowedFraction {
    /// Every strategy starts on paper. A default of anything else would be a
    /// decision nobody made about real money.
    fn default() -> Self {
        Zero
    }
}

/// Proof that a capital-authorising profile's gates all passed.
///
/// Not constructible outside this module: [`authorise`] is the only source, and
/// it refuses a profile that does not authorise capital. Holding one of these is
/// what it means to have earned a rung.
#[derive(Debug)]
pub struct RampAuthority {
    profile_id: String,
    gates_passed: usize,
}

impl RampAuthority {
    #[must_use]
    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    #[must_use]
    pub fn gates_passed(&self) -> usize {
        self.gates_passed
    }
}

/// Why a ramp was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RampRefusal {
    #[error("profile `{0}` does not authorise capital; a pass under it is forward-test evidence, not permission")]
    ProfileDoesNotAuthorise(String),
    #[error("{failed} of the sixteen gates did not pass under `{profile}`")]
    GatesFailed { profile: String, failed: usize },
    #[error("only {seen} of the sixteen gates were evaluated under `{profile}`; an unevaluated gate is not a passed one")]
    Incomplete { profile: String, seen: usize },
}

/// The number of gates §12.3 defines. All of them, every time: a promotion that
/// skipped one is a promotion nobody checked.
pub const GATE_COUNT: usize = 16;

/// Mint the authority to raise a rung.
///
/// `authorises_capital` comes from the gate profile row, not from the caller's
/// opinion of the profile. `verdicts` must cover all sixteen gates and all
/// sixteen must have passed.
///
/// # Errors
/// A profile that does not authorise capital, an incomplete gate stack, or any
/// gate that did not pass.
pub fn authorise(
    profile_id: &str,
    authorises_capital: bool,
    verdicts: &[(i32, bool)],
) -> Result<RampAuthority, RampRefusal> {
    if !authorises_capital {
        return Err(RampRefusal::ProfileDoesNotAuthorise(profile_id.to_string()));
    }
    let mut seen = [false; GATE_COUNT];
    let mut failed = 0_usize;
    for (gate_no, passed) in verdicts {
        if let Ok(i) = usize::try_from(*gate_no) {
            if (1..=GATE_COUNT).contains(&i) {
                seen[i - 1] = true;
                if !*passed {
                    failed += 1;
                }
            }
        }
    }
    let covered = seen.iter().filter(|s| **s).count();
    if covered < GATE_COUNT {
        return Err(RampRefusal::Incomplete { profile: profile_id.to_string(), seen: covered });
    }
    if failed > 0 {
        return Err(RampRefusal::GatesFailed { profile: profile_id.to_string(), failed });
    }
    Ok(RampAuthority { profile_id: profile_id.to_string(), gates_passed: covered })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_pass() -> Vec<(i32, bool)> {
        (1..=16).map(|g| (g, true)).collect()
    }

    #[test]
    fn the_ladder_is_the_five_values_spec_12_3_names() {
        let values: Vec<f64> = AllowedFraction::LADDER.iter().map(|f| f.value()).collect();
        assert_eq!(values, vec![0.0, 0.10, 0.25, 0.50, 1.0]);
        for f in AllowedFraction::LADDER {
            assert_eq!(AllowedFraction::from_code(f.as_str()), Some(f));
            assert_eq!(AllowedFraction::from_value(f.value()), Some(f));
        }
        // A value that is not a rung is not rounded onto one.
        assert_eq!(AllowedFraction::from_value(0.37), None);
        assert_eq!(AllowedFraction::from_value(0.11), None);
    }

    #[test]
    fn everything_starts_on_paper() {
        assert_eq!(AllowedFraction::default(), Zero);
    }

    /// AT-65 ⛔ — a `paper_v1` pass cannot raise the fraction above zero. There is
    /// no authority to be had, so there is no call that would do it.
    #[test]
    fn a_profile_that_does_not_authorise_capital_yields_no_authority() {
        let refused = authorise("paper_v1", false, &all_pass()).unwrap_err();
        assert!(matches!(refused, RampRefusal::ProfileDoesNotAuthorise(p) if p == "paper_v1"));

        // And the sixteen passes were real: the refusal is about the profile,
        // not about the evidence.
        let granted = authorise("strict_v1", true, &all_pass()).expect("strict_v1 authorises");
        assert_eq!(granted.gates_passed(), 16);
        assert_eq!(Zero.raise(&granted), Ten);
    }

    #[test]
    fn a_missing_gate_is_not_a_passed_gate() {
        let mut partial = all_pass();
        partial.pop();
        let err = authorise("strict_v1", true, &partial).unwrap_err();
        assert!(matches!(err, RampRefusal::Incomplete { seen: 15, .. }), "{err}");
    }

    #[test]
    fn one_failure_refuses_the_whole_stack() {
        let mut one_bad = all_pass();
        one_bad[7] = (8, false);
        let err = authorise("strict_v1", true, &one_bad).unwrap_err();
        assert!(matches!(err, RampRefusal::GatesFailed { failed: 1, .. }), "{err}");
    }

    #[test]
    fn a_ramp_climbs_one_rung_at_a_time_and_stops_at_the_top() {
        let a = authorise("strict_v1", true, &all_pass()).unwrap();
        let mut f = AllowedFraction::default();
        for expected in [Ten, TwentyFive, Fifty, Full, Full] {
            f = f.raise(&a);
            assert_eq!(f, expected);
        }
    }

    /// Going down needs nothing. A reduction that required authority is a
    /// reduction that happens after the loss rather than before it.
    #[test]
    fn stepping_down_needs_no_authority_and_halting_is_immediate() {
        assert_eq!(Full.step_down(), Fifty);
        assert_eq!(Fifty.step_down(), TwentyFive);
        assert_eq!(Zero.step_down(), Zero);
        assert_eq!(AllowedFraction::halt(), Zero);
    }

    #[test]
    fn the_rungs_are_ordered_the_way_they_read() {
        assert!(Zero < Ten && Ten < TwentyFive && TwentyFive < Fifty && Fifty < Full);
    }
}
