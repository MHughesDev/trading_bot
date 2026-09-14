//! The only as-of join in the platform (SPEC §2 rule 2, INV-10).
//!
//! Backward-only. There is no direction parameter and no `nearest` variant, so a
//! forward or nearest match is not merely discouraged — it is unrepresentable.
//! `ci_invariants::asof_nearest_is_unreachable` fails the build if any other
//! as-of join appears in the workspace.

/// For each `left` timestamp, the index of the latest `right` entry whose timestamp
/// is `<=` it, optionally bounded by `tolerance` (same unit as the keys).
///
/// `right_keys` must be sorted ascending.
#[must_use]
pub fn asof_backward(left_keys: &[i64], right_keys: &[i64], tolerance: Option<i64>) -> Vec<Option<usize>> {
    debug_assert!(right_keys.windows(2).all(|w| w[0] <= w[1]), "right_keys must be sorted");
    left_keys
        .iter()
        .map(|&t| {
            let pos = right_keys.partition_point(|&r| r <= t);
            if pos == 0 {
                return None;
            }
            let idx = pos - 1;
            match tolerance {
                Some(tol) if t - right_keys[idx] > tol => None,
                _ => Some(idx),
            }
        })
        .collect()
}

/// Strategy names that callers sometimes reach for. Rejected by name so a caller
/// porting code from a dataframe library gets a precise error instead of silent
/// look-ahead.
///
/// # Errors
/// Always rejects anything other than `"backward"`.
pub fn parse_strategy(name: &str) -> Result<(), String> {
    match name {
        "backward" => Ok(()),
        "nearest" | "forward" => Err(format!(
            "asof strategy '{name}' is look-ahead and is banned (SPEC §2, INV-10); only 'backward' exists"
        )),
        other => Err(format!("unknown asof strategy '{other}'; only 'backward' exists")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_matches_the_future() {
        let left = [5, 10, 15, 20];
        let right = [0, 9, 16];
        assert_eq!(asof_backward(&left, &right, None), vec![Some(0), Some(1), Some(1), Some(2)]);
        // 15 is closer to 16 than 9, but 16 is in its future.
        assert_eq!(asof_backward(&[15], &right, None), vec![Some(1)]);
    }

    #[test]
    fn tolerance_bounds_staleness() {
        assert_eq!(asof_backward(&[100], &[10], Some(50)), vec![None]);
        assert_eq!(asof_backward(&[100], &[60], Some(50)), vec![Some(0)]);
        assert_eq!(asof_backward(&[1], &[2], None), vec![None]);
    }

    /// AT-10 (wrapper half): the wrapper rejects `nearest`.
    #[test]
    fn nearest_is_rejected() {
        assert!(parse_strategy("nearest").is_err());
        assert!(parse_strategy("forward").is_err());
        assert!(parse_strategy("backward").is_ok());
    }
}
