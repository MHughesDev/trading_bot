//! Gate profiles: versioned, immutable threshold sets (SPEC §12.3, INV-23).
//!
//! **Changing a threshold creates a new profile; it never edits an existing
//! one.** That is enforced in two places, because one is not enough:
//!
//! * in the database, by a trigger that refuses UPDATE and DELETE on
//!   `mlops.gate_profile` (migration 0043), and by no agent holding a grant on
//!   the table at all (INV-23);
//! * in this type, by having no setter and no mutable accessor — a profile is
//!   constructed whole or not constructed.
//!
//! The reason is threshold drift. A gate stack whose numbers can be edited in
//! place silently rewrites every historical verdict: a strategy that passed last
//! year under `dsr ≥ 0.95` and would fail today under `dsr ≥ 0.99` still reads
//! as "passed", with nothing on the record saying which bar it cleared. So every
//! verdict carries the `profile_id` it was judged under, and a comparison that
//! spans two profiles is [`Comparability::NonComparable`] — reported, not
//! silently averaged.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The canonical profile id shipped with the platform. The only one that
/// authorises capital.
pub const STRICT_V1: &str = "strict_v1";

/// `strict_v1` minus Gate 9's calendar floor and minus Gate 16 (ADR-P2-14).
///
/// It exists because a platform whose history is months old cannot pass a
/// five-year floor, and that floor is *right* — a short backtest plus many trials
/// is indistinguishable from noise. But paper is where the years are honestly
/// accumulated, and a strategy that cannot reach paper never accumulates them.
/// What bounds the risk is that `paper_v1` cannot authorise capital at all.
pub const PAPER_V1: &str = "paper_v1";

/// The thresholds of SPEC §12.3's sixteen gates.
///
/// **No serde defaults.** A profile that omits a threshold fails to parse rather
/// than silently inheriting one: an unstated bar is not a permissive bar, it is
/// an unknown bar, and a gate stack cannot run against an unknown bar.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    /// Gate 1 — pre-registration hash-locked before the first backtest.
    pub preregistration_required: bool,
    /// Gate 2 — fraction of the leakage suite that must pass. 1.0 in `strict_v1`.
    pub leakage_suite_pass_rate: f64,
    /// Gate 3 — breakeven cost multiple.
    pub min_breakeven_cost_multiple: f64,
    /// Gate 4 — capacity.
    pub max_capacity_fraction_at_half_sharpe: f64,
    pub adv_soft: f64,
    pub adv_hard: f64,
    /// Gate 5 — CPCV 5th-percentile path Sharpe.
    pub cpcv_p05_sharpe_gt: f64,
    /// Gate 6 — strictly-causal walk-forward.
    pub walk_forward_sharpe_gt: f64,
    pub walk_forward_min_regimes: u32,
    /// Gate 7 — probability of backtest overfitting.
    pub pbo_lt: f64,
    /// Gate 8 — deflated Sharpe on platform-counted `N_eff`.
    pub dsr_gte: f64,
    /// Gate 9 — minimum backtest length.
    pub min_track_record_years: f64,
    pub min_independent_events: u32,
    /// Gate 10 — factor attribution.
    pub alpha_t_stat_gte: f64,
    pub factor_r2_lt: f64,
    /// Gate 11 — regime coverage.
    pub max_single_regime_pnl_share: f64,
    /// Gate 12 — perturbation robustness.
    pub max_single_instrument_pnl_share: f64,
    /// Gate 13 — stationary bootstrap.
    pub bootstrap_p05_sharpe_gt: f64,
    /// Gate 14 — Romano–Wolf stepdown against the full candidate family.
    pub romano_wolf_p_lt: f64,
    /// Gate 15 — paper/shadow process gates (§12.6).
    pub paper_signal_match_gte: f64,
    pub paper_slippage_ratio_lte: f64,
    pub paper_turnover_tolerance: f64,
    pub paper_model_rejects_max: u32,
}

/// One immutable, versioned threshold set.
///
/// There is no constructor that takes a `profile_id` and mutable thresholds, and
/// no method that changes one: a new set of numbers is a new profile, named by
/// the caller and pointing at what it supersedes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GateProfile {
    profile_id: String,
    supersedes: Option<String>,
    thresholds: Thresholds,
    /// Whether a pass under this profile may raise a strategy's capital
    /// allocation. A schema column, not a convention: `paper_v1` is `false` and
    /// Gate 16 refuses on it (AT-65).
    authorises_capital: bool,
    /// Gate 10's factor battery and Gate 11's crisis windows, per asset class
    /// (ADR-P2-15, P2-16). Empty when the profile has not been loaded from the
    /// database; `battery_for` then reports the asset class as undeclared rather
    /// than guessing one.
    #[serde(default)]
    asset_classes: BTreeMap<String, AssetClassGates>,
}

/// The per-asset-class half of a profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssetClassGates {
    /// Gate 10. Factor names the attribution regression runs against.
    pub factor_battery: Vec<String>,
    /// Gate 11. Dated episodes that count as crisis windows for this asset class.
    pub crisis_windows: Vec<CrisisWindow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrisisWindow {
    pub label: String,
    pub from: chrono::NaiveDate,
    pub to: chrono::NaiveDate,
}

impl GateProfile {
    /// The profile shipped with the platform (migration 0043 inserts the same
    /// numbers, and `gate_profile_matches_the_shipped_row` keeps the two honest).
    #[must_use]
    pub fn strict_v1() -> Self {
        Self {
            profile_id: STRICT_V1.to_string(),
            supersedes: None,
            authorises_capital: true,
            asset_classes: BTreeMap::new(),
            thresholds: Thresholds {
                preregistration_required: true,
                leakage_suite_pass_rate: 1.0,
                min_breakeven_cost_multiple: 3.0,
                max_capacity_fraction_at_half_sharpe: 0.20,
                adv_soft: 0.05,
                adv_hard: 0.10,
                cpcv_p05_sharpe_gt: 0.0,
                walk_forward_sharpe_gt: 0.0,
                walk_forward_min_regimes: 3,
                pbo_lt: 0.20,
                dsr_gte: 0.95,
                min_track_record_years: 5.0,
                min_independent_events: 300,
                alpha_t_stat_gte: 3.0,
                factor_r2_lt: 0.7,
                max_single_regime_pnl_share: 0.50,
                max_single_instrument_pnl_share: 0.20,
                bootstrap_p05_sharpe_gt: 0.0,
                romano_wolf_p_lt: 0.05,
                paper_signal_match_gte: 0.99,
                paper_slippage_ratio_lte: 1.5,
                paper_turnover_tolerance: 0.20,
                paper_model_rejects_max: 0,
            },
        }
    }

    /// `paper_v1`: the shipped profile minus Gate 9's calendar floor, and unable to
    /// authorise capital. Kept identical to migration 0050's row by a test.
    #[must_use]
    pub fn paper_v1() -> Self {
        let base = Self::strict_v1();
        let mut t = *base.thresholds();
        t.min_track_record_years = 0.0;
        Self {
            profile_id: PAPER_V1.to_string(),
            supersedes: Some(STRICT_V1.to_string()),
            thresholds: t,
            authorises_capital: false,
            asset_classes: BTreeMap::new(),
        }
    }

    /// A new version of an existing profile. The only way to change a threshold.
    ///
    /// A superseding profile inherits its predecessor's capital authority and its
    /// per-asset-class facts; widening authority is an explicit act
    /// ([`Self::authorising_capital`]), never a side effect of changing a number.
    #[must_use]
    pub fn superseding(profile_id: impl Into<String>, base: &Self, thresholds: Thresholds) -> Self {
        Self {
            profile_id: profile_id.into(),
            supersedes: Some(base.profile_id.clone()),
            thresholds,
            authorises_capital: base.authorises_capital,
            asset_classes: base.asset_classes.clone(),
        }
    }

    /// Declare that this profile authorises capital. Explicit by design.
    #[must_use]
    pub fn authorising_capital(mut self, yes: bool) -> Self {
        self.authorises_capital = yes;
        self
    }

    /// Attach the per-asset-class gate facts loaded from
    /// `mlops.gate_profile_asset_class`.
    #[must_use]
    pub fn with_asset_class(mut self, asset_class: impl Into<String>, gates: AssetClassGates) -> Self {
        self.asset_classes.insert(asset_class.into(), gates);
        self
    }

    /// Whether a pass under this profile can move a strategy up the capital
    /// ramp at all (§12.3 Gate 16). `paper_v1` cannot: the evidence it
    /// accumulates is what `strict_v1` will later need, not permission in its
    /// own right.
    #[must_use]
    pub fn authorises_capital(&self) -> bool {
        self.authorises_capital
    }

    /// Gate 10 and 11's facts for an asset class.
    ///
    /// `None` means this profile has not declared them. Callers must treat that as
    /// "the gate cannot run", never as "the gate passes": a factor regression with
    /// no factors is not attribution, and a crisis-window count of zero from an
    /// undeclared list is not coverage.
    #[must_use]
    pub fn asset_class(&self, asset_class: &str) -> Option<&AssetClassGates> {
        self.asset_classes.get(asset_class)
    }

    /// Parse a stored profile. A `thresholds` object missing any field is an
    /// error, not a partially-defaulted profile.
    ///
    /// # Errors
    /// Missing or mistyped thresholds.
    pub fn from_row(
        profile_id: impl Into<String>,
        supersedes: Option<String>,
        thresholds: &serde_json::Value,
    ) -> Result<Self, serde_json::Error> {
        Ok(Self {
            profile_id: profile_id.into(),
            supersedes,
            thresholds: serde_json::from_value(thresholds.clone())?,
            authorises_capital: true,
            asset_classes: BTreeMap::new(),
        })
    }

    #[must_use]
    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    #[must_use]
    pub fn supersedes(&self) -> Option<&str> {
        self.supersedes.as_deref()
    }

    #[must_use]
    pub fn thresholds(&self) -> &Thresholds {
        &self.thresholds
    }
}

/// Whether two gated results may be put side by side.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Comparability {
    /// Both results were judged under the same profile.
    Comparable { profile_id: String },
    /// They were not. The comparison is still *shown* — hiding it would be its
    /// own kind of dishonesty — but it is labelled, because a pass under one bar
    /// and a pass under another are not the same claim.
    NonComparable { left: String, right: String },
}

impl Comparability {
    #[must_use]
    pub fn of(left: &str, right: &str) -> Self {
        if left == right {
            Self::Comparable {
                profile_id: left.to_string(),
            }
        } else {
            Self::NonComparable {
                left: left.to_string(),
                right: right.to_string(),
            }
        }
    }

    #[must_use]
    pub fn is_comparable(&self) -> bool {
        matches!(self, Self::Comparable { .. })
    }

    /// The line a comparison view renders when the two sides do not share a bar.
    #[must_use]
    pub fn notice(&self) -> Option<String> {
        match self {
            Self::Comparable { .. } => None,
            Self::NonComparable { left, right } => Some(format!(
                "non-comparable: judged under gate profiles {left} and {right}. A pass under one \
                 threshold set is not a pass under the other."
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AT-29 (unit half): the shipped profile has no mutator. Changing a
    /// threshold produces a *new* profile that names what it supersedes.
    #[test]
    fn changing_a_threshold_creates_a_new_version_and_names_its_predecessor() {
        let v1 = GateProfile::strict_v1();
        let mut t = *v1.thresholds();
        t.dsr_gte = 0.99;
        let v2 = GateProfile::superseding("strict_v2", &v1, t);

        assert_eq!(v2.profile_id(), "strict_v2");
        assert_eq!(v2.supersedes(), Some(STRICT_V1));
        assert!((v2.thresholds().dsr_gte - 0.99).abs() < f64::EPSILON);
        // v1 is untouched: the old verdicts still mean what they meant.
        assert!((v1.thresholds().dsr_gte - 0.95).abs() < f64::EPSILON);
    }

    /// `paper_v1` differs from `strict_v1` in exactly two ways, and one of them is
    /// that it cannot authorise capital (AT-65).
    #[test]
    fn paper_v1_drops_the_calendar_floor_and_cannot_authorise_capital() {
        let strict = GateProfile::strict_v1();
        let paper = GateProfile::paper_v1();

        assert!(strict.authorises_capital());
        assert!(!paper.authorises_capital(), "paper_v1 must never authorise capital");
        assert_eq!(paper.supersedes(), Some(STRICT_V1));

        // Exactly one threshold differs, and it is the calendar floor.
        let (a, b) = (*strict.thresholds(), *paper.thresholds());
        assert!((b.min_track_record_years - 0.0).abs() < f64::EPSILON);
        assert!((a.min_track_record_years - 5.0).abs() < f64::EPSILON);
        let mut a_rest = a;
        a_rest.min_track_record_years = 0.0;
        assert_eq!(a_rest, b, "paper_v1 relaxes the calendar floor and nothing else");

        // The statistical floors it keeps are the ones P-01 is about.
        assert_eq!(b.min_independent_events, 300);
        assert!((b.dsr_gte - 0.95).abs() < f64::EPSILON);
    }

    /// An undeclared asset class reports as undeclared. A gate that cannot find
    /// its factors must not read that as a pass.
    #[test]
    fn an_undeclared_asset_class_is_none_not_an_empty_battery() {
        let p = GateProfile::strict_v1();
        assert!(p.asset_class("crypto").is_none());

        let p = p.with_asset_class(
            "crypto",
            AssetClassGates {
                factor_battery: vec!["cmkt".into(), "cmom".into()],
                crisis_windows: Vec::new(),
            },
        );
        assert_eq!(p.asset_class("crypto").unwrap().factor_battery.len(), 2);
        assert!(p.asset_class("equity").is_none());
    }

    /// A superseding profile inherits capital authority rather than acquiring it.
    #[test]
    fn superseding_a_paper_profile_does_not_grant_capital_authority() {
        let paper = GateProfile::paper_v1();
        let mut t = *paper.thresholds();
        t.dsr_gte = 0.99;
        let v2 = GateProfile::superseding("paper_v2", &paper, t);
        assert!(!v2.authorises_capital());
        assert!(v2.authorising_capital(true).authorises_capital());
    }

    #[test]
    fn results_under_different_profiles_are_not_comparable() {
        let same = Comparability::of(STRICT_V1, STRICT_V1);
        assert!(same.is_comparable());
        assert!(same.notice().is_none());

        let spanning = Comparability::of(STRICT_V1, "strict_v2");
        assert!(!spanning.is_comparable());
        assert!(spanning.notice().unwrap().contains("non-comparable"));
    }

    /// No serde default means an incomplete profile does not parse. A threshold
    /// nobody stated is unknown, not permissive.
    #[test]
    fn a_profile_missing_a_threshold_does_not_parse() {
        let partial = serde_json::json!({ "dsr_gte": 0.95, "pbo_lt": 0.20 });
        assert!(GateProfile::from_row("broken", None, &partial).is_err());
    }

    #[test]
    fn a_complete_profile_round_trips_through_the_stored_shape() {
        let v1 = GateProfile::strict_v1();
        let json = serde_json::to_value(v1.thresholds()).expect("serialize");
        let back = GateProfile::from_row(STRICT_V1, None, &json).expect("parse");
        assert_eq!(&back, &v1);
    }

    /// The Rust constant and the row migration 0043 inserts must be the same
    /// numbers, or the gate stack runs against thresholds the database does not
    /// have on record.
    /// The `paper_v1` constant and migration 0050's row must be the same numbers,
    /// or a verdict recorded by the database was judged against thresholds the
    /// code does not have.
    #[test]
    fn paper_v1_matches_the_row_the_migration_inserts() {
        let sql = include_str!("../../../../migrations/0050_gates_v2.sql");
        let start = sql
            .find("INSERT INTO mlops.gate_profile (profile_id, thresholds")
            .expect("migration 0050 inserts paper_v1");
        let open = sql[start..].find('{').expect("a json object") + start;
        let close = sql[open..].find("}'::jsonb").expect("the object ends") + open + 1;
        let stored: serde_json::Value =
            serde_json::from_str(&sql[open..close]).expect("valid json in the migration");

        let mine = serde_json::to_value(GateProfile::paper_v1().thresholds()).expect("serialize");
        for (k, v) in mine.as_object().expect("object") {
            assert_eq!(
                stored.get(k),
                Some(v),
                "threshold {k} differs between migration 0050 and GateProfile::paper_v1"
            );
        }
        assert!(
            sql[start..].contains("FALSE)"),
            "paper_v1 must be inserted with authorises_capital = FALSE"
        );
    }

    #[test]
    fn strict_v1_matches_the_row_the_migration_inserts() {
        let sql = include_str!("../../../../migrations/0043_trial_ledger.sql");
        let start = sql
            .find("INSERT INTO mlops.gate_profile")
            .expect("the migration inserts strict_v1");
        let open = sql[start..].find('{').expect("a json object") + start;
        let close = sql[open..].find("}'::jsonb").expect("the object ends") + open + 1;
        let stored: serde_json::Value =
            serde_json::from_str(&sql[open..close]).expect("valid json in the migration");

        let mine = serde_json::to_value(GateProfile::strict_v1().thresholds()).expect("serialize");
        for (k, v) in mine.as_object().expect("object") {
            assert_eq!(
                stored.get(k),
                Some(v),
                "threshold {k} differs between the migration and GateProfile::strict_v1"
            );
        }
    }
}
