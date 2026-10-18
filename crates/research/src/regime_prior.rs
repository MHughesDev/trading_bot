//! What the regime engine tells the search loop (Set R-b RB-4.3 to RB-4.5; FEAT-003 §9,
//! §10, §11).
//!
//! Three small pure pieces over *filtered* labels:
//!
//! * [`profile`]: occupancy, transitions and the current label of a label series, for
//!   the `get_regime_profile` tool. An instrument with no labels is a typed refusal,
//!   not an empty profile: those mean opposite things.
//! * [`PriorCell`] and [`top_families`]: the `(instrument, regime, strategy_family)`
//!   table as a *contextual-bandit table*, not a classifier. After each candidate's
//!   regime-sliced study it is updated with that regime's score; the first-generation
//!   hypothesis of a campaign can cite the top families with their observation counts.
//! * [`resolve_regime_gate`]: the `add_regime_gate` move's machine check. The move
//!   names a regime label; the check refuses a label the engine does not know and
//!   returns the revision's regime-source citation.
//!
//! None of these reads the future: they consume labels already stored as filtered
//! (ADR-RB-05) and per-regime study scores already measured.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::propose::Move;

// ── get_regime_profile ──────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ProfileError {
    #[error("no regime labels exist for {0:?}; run the labeller for its scope before asking for a profile")]
    NoLabels(String),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RegimeProfile {
    pub instrument: String,
    pub n: usize,
    pub current: String,
    /// Share of observations in each regime.
    pub occupancy: BTreeMap<String, f64>,
    /// `transitions[from][to]` as a row-normalised share of departures, including
    /// staying. A regime never left has a row of `1.0` on itself.
    pub transitions: BTreeMap<String, BTreeMap<String, f64>>,
    /// Median length of a stay, in observations.
    pub median_run: f64,
}

/// The profile of a filtered label series, oldest first.
///
/// # Errors
/// [`ProfileError::NoLabels`] for an empty series.
pub fn profile(instrument: &str, labels: &[String]) -> Result<RegimeProfile, ProfileError> {
    let Some(last) = labels.last() else {
        return Err(ProfileError::NoLabels(instrument.to_string()));
    };
    let n = labels.len() as f64;
    let mut occ: BTreeMap<String, f64> = BTreeMap::new();
    for l in labels {
        *occ.entry(l.clone()).or_default() += 1.0 / n;
    }
    let mut counts: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    for w in labels.windows(2) {
        *counts
            .entry(w[0].clone())
            .or_default()
            .entry(w[1].clone())
            .or_default() += 1.0;
    }
    let transitions = labels
        .iter()
        .map(|l| {
            let row = counts.get(l).cloned().unwrap_or_default();
            let total: f64 = row.values().sum();
            let norm: BTreeMap<String, f64> = if total > 0.0 {
                row.into_iter().map(|(k, v)| (k, v / total)).collect()
            } else {
                BTreeMap::from([(l.clone(), 1.0)])
            };
            (l.clone(), norm)
        })
        .collect();
    let mut runs: Vec<usize> = Vec::new();
    let mut len = 1usize;
    for w in labels.windows(2) {
        if w[0] == w[1] {
            len += 1;
        } else {
            runs.push(len);
            len = 1;
        }
    }
    runs.push(len);
    runs.sort_unstable();
    let m = runs.len();
    let median_run = if m % 2 == 1 {
        runs[m / 2] as f64
    } else {
        (runs[m / 2 - 1] + runs[m / 2]) as f64 / 2.0
    };
    Ok(RegimeProfile {
        instrument: instrument.to_string(),
        n: labels.len(),
        current: last.clone(),
        occupancy: occ,
        transitions,
        median_run,
    })
}

// ── the prior table (RB-4.4) ────────────────────────────────────────────────

/// One cell of the `(instrument, regime, strategy_family)` table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PriorCell {
    pub n_obs: u32,
    pub n_success: u32,
    pub sum: f64,
    pub sumsq: f64,
}

impl PriorCell {
    /// Fold one regime-sliced study into the cell. `success` is the study's own
    /// verdict in that regime; `score` is its sealed-metric value.
    pub fn update(&mut self, score: f64, success: bool) {
        if !score.is_finite() {
            return;
        }
        self.n_obs += 1;
        self.n_success += u32::from(success);
        self.sum += score;
        self.sumsq += score * score;
    }

    #[must_use]
    pub fn mean(&self) -> Option<f64> {
        (self.n_obs > 0).then(|| self.sum / f64::from(self.n_obs))
    }

    /// An upper confidence bound `mean + c / sqrt(n)`: a bandit's optimism, so an
    /// untried family is *explored* rather than ranked last. `None` with no data.
    #[must_use]
    pub fn ucb(&self, c: f64) -> Option<f64> {
        self.mean().map(|m| m + c / f64::from(self.n_obs).sqrt())
    }
}

/// The families worth trying first in a regime: by UCB, each with its observation
/// count so a reader can see how much a number rests on. Cells with no data are
/// listed last with `n_obs = 0`.
#[must_use]
pub fn top_families(
    cells: &BTreeMap<String, PriorCell>,
    k: usize,
    c: f64,
) -> Vec<(String, Option<f64>, u32)> {
    let mut rows: Vec<(String, Option<f64>, u32)> = cells
        .iter()
        .map(|(f, cell)| (f.clone(), cell.ucb(c), cell.n_obs))
        .collect();
    rows.sort_by(|a, b| match (a.1, b.1) {
        (Some(x), Some(y)) => y.total_cmp(&x).then_with(|| a.0.cmp(&b.0)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.0.cmp(&b.0),
    });
    rows.truncate(k);
    rows
}

// ── add_regime_gate (RB-4.5) ────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum RegimeGateError {
    #[error("this is not an add_regime_gate move")]
    WrongMove,
    #[error("{0:?} is not a regime label the engine knows; known labels: {1:?}")]
    UnknownLabel(String, Vec<String>),
    #[error(
        "a regime gate needs a regime source to cite (the model version that labelled the history)"
    )]
    NoSource,
}

/// The validated gate and the citation it carries.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RegimeGate {
    pub regime: String,
    /// `regime_causal.regime_state` at a model version: what the gate reads.
    pub source: String,
}

/// The `Pending::RegimeLabels` check for an `add_regime_gate` move.
///
/// # Errors
/// [`RegimeGateError`].
pub fn resolve_regime_gate(
    m: &Move,
    known_labels: &[&str],
    source: &str,
) -> Result<RegimeGate, RegimeGateError> {
    let Move::AddRegimeGate { regime } = m else {
        return Err(RegimeGateError::WrongMove);
    };
    if source.trim().is_empty() {
        return Err(RegimeGateError::NoSource);
    }
    if !known_labels.contains(&regime.as_str()) {
        return Err(RegimeGateError::UnknownLabel(
            regime.clone(),
            known_labels.iter().map(|s| (*s).to_string()).collect(),
        ));
    }
    Ok(RegimeGate {
        regime: regime.clone(),
        source: format!("regime_causal.regime_state@{source}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn the_profile_reports_occupancy_transitions_and_the_current_label() {
        let l = labels("low low low mid mid high low low");
        let p = profile("BTC-USD", &l).unwrap();
        assert_eq!(p.n, 8);
        assert_eq!(p.current, "low");
        assert!((p.occupancy["low"] - 5.0 / 8.0).abs() < 1e-12);
        assert!((p.occupancy.values().sum::<f64>() - 1.0).abs() < 1e-12);
        // From "low": three stays and one move to "mid", so a quarter of departures.
        assert!((p.transitions["low"]["mid"] - 0.25).abs() < 1e-12);
        assert!((p.transitions["low"].values().sum::<f64>() - 1.0).abs() < 1e-12);
        assert_eq!(p.median_run, 2.0);
    }

    #[test]
    fn an_instrument_with_no_labels_is_a_typed_refusal() {
        assert_eq!(profile("X", &[]), Err(ProfileError::NoLabels("X".into())));
    }

    #[test]
    fn a_regime_never_left_stays_with_certainty() {
        let p = profile("X", &labels("a a a a")).unwrap();
        assert_eq!(p.transitions["a"]["a"], 1.0);
    }

    #[test]
    fn a_study_updates_its_own_cell_and_the_prior_reports_its_observation_count() {
        let mut cells: BTreeMap<String, PriorCell> = BTreeMap::new();
        cells.entry("trend".into()).or_default().update(1.2, true);
        cells.entry("trend".into()).or_default().update(0.8, true);
        cells
            .entry("mean_reversion".into())
            .or_default()
            .update(-0.2, false);
        cells.entry("carry".into()).or_default();
        cells
            .entry("trend".into())
            .or_default()
            .update(f64::NAN, true); // ignored
        assert_eq!(cells["trend"].n_obs, 2);
        assert!((cells["trend"].mean().unwrap() - 1.0).abs() < 1e-12);
        let top = top_families(&cells, 3, 0.5);
        assert_eq!(top[0].0, "trend");
        assert_eq!(top[0].2, 2, "n_obs travels with the number");
        assert_eq!(
            top.last().unwrap().0,
            "carry",
            "an untried family is listed last with n_obs 0"
        );
        assert_eq!(top.last().unwrap().2, 0);
    }

    #[test]
    fn optimism_lets_a_thinly_tried_family_outrank_a_slightly_better_well_tried_one() {
        let mut well = PriorCell::default();
        for _ in 0..100 {
            well.update(0.50, true);
        }
        let mut thin = PriorCell::default();
        thin.update(0.45, true);
        assert!(thin.ucb(0.5).unwrap() > well.ucb(0.5).unwrap());
    }

    #[test]
    fn an_unknown_regime_label_is_refused_and_a_known_one_cites_its_source() {
        let known = ["low_vol", "mid_vol", "high_vol"];
        let ok = Move::AddRegimeGate {
            regime: "high_vol".into(),
        };
        let g = resolve_regime_gate(&ok, &known, "jump_v1:2026-10").unwrap();
        assert_eq!(g.source, "regime_causal.regime_state@jump_v1:2026-10");
        let bad = Move::AddRegimeGate {
            regime: "bubble".into(),
        };
        assert!(matches!(
            resolve_regime_gate(&bad, &known, "x"),
            Err(RegimeGateError::UnknownLabel(..))
        ));
        assert_eq!(
            resolve_regime_gate(&ok, &known, " "),
            Err(RegimeGateError::NoSource)
        );
        assert_eq!(
            resolve_regime_gate(&Move::Simplify, &known, "x"),
            Err(RegimeGateError::WrongMove)
        );
    }
}
