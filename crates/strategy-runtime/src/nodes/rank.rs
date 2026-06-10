//! Rank node — sorts the universe by a named feature value.

use crate::nodes::Universe;

/// Sort `universe` by `feature`.
///
/// Instruments missing the feature are pushed to the end regardless of
/// `ascending`.  Ties preserve original relative order (stable sort).
pub fn rank(mut universe: Universe, feature: &str, ascending: bool) -> Universe {
    universe.sort_by(|a, b| {
        let av = a
            .features
            .get(feature)
            .copied()
            .unwrap_or(f64::NEG_INFINITY);
        let bv = b
            .features
            .get(feature)
            .copied()
            .unwrap_or(f64::NEG_INFINITY);
        let ord = av.partial_cmp(&bv).unwrap_or(std::cmp::Ordering::Equal);
        if ascending {
            ord
        } else {
            ord.reverse()
        }
    });
    universe
}
