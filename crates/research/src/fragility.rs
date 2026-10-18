//! One number for how knife-edged a sweep's winner is (S-3.13
//! `parameter_fragility_map`, AGENT-006 §5.5).
//!
//! # Re-scoped, on purpose
//!
//! The plan asked for a tool that perturbs parameters and re-runs. That would be
//! a second answer to arithmetic the platform already performs: [`crate::surface`]
//! computes plateaus, cliffs and sensitivity per parameter from the sweep's own
//! samples, and the neighbourhood Study already carries a plateau-versus-knife-
//! edge verdict (`SweepReport::neighbourhood_plateau`). What was missing is the
//! single scalar and the dispersion it should be read beside. This is that, over
//! a finished sweep, and it runs nothing: no simulator, no trial.
//!
//! # The definition (a convention, stated)
//!
//! A parameter is **knife-edge** when its marginal surface has a cliff, or has no
//! plateau while its sensitivity is not low. `fragility_index` is the share of
//! parameters that are knife-edge, in `[0, 1]`. It is an index of how many of the
//! dials the result is balanced on, not a probability, and it is not compared with
//! any threshold here: the verdict is the neighbourhood Study's, and the index is
//! reported beside it.
//!
//! `neighbourhood_dispersion_ratio` is `spread / |median|` over the sealed
//! neighbourhood distribution -- the quantity Set J's plateau verdict thresholds at
//! 0.5 -- so the reader sees how far from that line the sweep sits.

use backtest::study::result::Distribution;
use serde::Serialize;

use crate::surface::{ParamSurface, Sensitivity, SurfaceSummary};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ParamFragility {
    pub name: String,
    pub knife_edge: bool,
    pub has_cliff: bool,
    pub has_plateau: bool,
    pub sensitivity: Sensitivity,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Fragility {
    pub fragility_index: Option<f64>,
    pub n_params: usize,
    pub n_knife_edge: usize,
    pub per_param: Vec<ParamFragility>,
    pub neighbourhood_plateau: Option<bool>,
    pub neighbourhood_dispersion_ratio: Option<f64>,
    /// Always `None`: the carried-forward point's own score is not on the report,
    /// so its percentile among the samples cannot be computed from here.
    pub chosen_point_percentile: Option<f64>,
    pub chosen_point_percentile_note: &'static str,
}

fn param(p: &ParamSurface) -> ParamFragility {
    let has_cliff = !p.cliffs.is_empty();
    let has_plateau = p.plateau.is_some();
    let knife_edge = has_cliff || (!has_plateau && p.sensitivity != Sensitivity::Low);
    ParamFragility {
        name: p.name.clone(),
        knife_edge,
        has_cliff,
        has_plateau,
        sensitivity: p.sensitivity,
    }
}

/// Reduce a finished sweep's surface and neighbourhood to the index.
#[must_use]
pub fn fragility(
    surface: &SurfaceSummary,
    neighbourhood: Option<bool>,
    distribution: &Distribution,
) -> Fragility {
    let per_param: Vec<ParamFragility> = surface.per_param.iter().map(param).collect();
    let n_knife_edge = per_param.iter().filter(|p| p.knife_edge).count();
    let n = per_param.len();
    let ratio = (distribution.median.abs() > 0.0 && !distribution.dist.is_empty())
        .then(|| distribution.spread / distribution.median.abs());
    Fragility {
        // No parameters, no index: a sweep over nothing is not a robust one.
        fragility_index: (n > 0).then(|| n_knife_edge as f64 / n as f64),
        n_params: n,
        n_knife_edge,
        per_param,
        neighbourhood_plateau: neighbourhood,
        neighbourhood_dispersion_ratio: ratio,
        chosen_point_percentile: None,
        chosen_point_percentile_note: "the carried-forward point's own score is not recorded on the sweep report, so its percentile cannot be computed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::Bin;
    use backtest::run::MetricKind;

    fn surf(name: &str, plateau: bool, cliffs: usize, s: Sensitivity) -> ParamSurface {
        ParamSurface {
            name: name.into(),
            bins: vec![Bin { label: "a".into(), mean: 1.0, n: 3 }],
            plateau: plateau.then(|| ("a".into(), "b".into())),
            cliffs: (0..cliffs).map(|i| format!("c{i}")).collect(),
            sensitivity: s,
        }
    }

    fn summary(params: Vec<ParamSurface>) -> SurfaceSummary {
        SurfaceSummary { n_samples: 10, n_finite: 10, per_param: params, text: String::new() }
    }

    fn dist(values: Vec<f64>) -> Distribution {
        Distribution::from_values(MetricKind::Sharpe, values)
    }

    #[test]
    fn a_cliff_or_a_sensitive_plateauless_parameter_is_knife_edge() {
        let s = summary(vec![
            surf("a", true, 0, Sensitivity::Low),
            surf("b", true, 1, Sensitivity::Medium),
            surf("c", false, 0, Sensitivity::High),
            surf("d", false, 0, Sensitivity::Low),
        ]);
        let f = fragility(&s, Some(false), &dist(vec![1.0, 1.1, 0.9]));
        assert_eq!(f.n_knife_edge, 2);
        assert_eq!(f.fragility_index, Some(0.5));
    }

    /// A sweep over no parameters has no index; it must not read as zero.
    #[test]
    fn no_parameters_means_no_index() {
        let f = fragility(&summary(vec![]), None, &dist(vec![]));
        assert_eq!(f.fragility_index, None);
        assert_eq!(f.neighbourhood_dispersion_ratio, None);
    }

    #[test]
    fn the_dispersion_ratio_is_the_one_the_plateau_verdict_uses() {
        let f = fragility(&summary(vec![surf("a", true, 0, Sensitivity::Low)]), Some(true), &dist(vec![1.0, 1.0, 1.0, 1.2]));
        let r = f.neighbourhood_dispersion_ratio.unwrap();
        assert!(r > 0.0 && r < 0.5, "{r}");
    }
}
