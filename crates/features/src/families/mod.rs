//! The F2 families the engine was missing (Set S S-5.14; AGENT-011 §2 F2).
//!
//! # What was missing, and why these four
//!
//! The reconciliation test of 2026-09-16 (`volatility_reconciliation.rs`) found
//! that the F2 set is *not* consistent across the language barrier:
//!
//! | family | engine | analysis templates |
//! |---|---|---|
//! | `yang_zhang` | **missing** | `estimate_volatility.py` |
//! | `hurst` | **missing** | `detect_regime.py` |
//! | `efficiency_ratio` | `runtime.rs` | **missing** (added: `detect_regime.py`) |
//! | `fracdiff` | **missing** | **missing** (added: `stationarity.py`) |
//! | `lookback_ensemble` | **missing** | none — see below |
//!
//! A family that exists once cannot drift, which is why those were recorded as
//! gaps and not as failures. They are still gaps: AC-1 says every feature the
//! templates compute is one a strategy can read.
//!
//! # Pure functions, and where they plug in
//!
//! Each module is a pure function of slices. No registry, no `Feature`, no I/O,
//! no `crate::` path — so `builders-purity` is untouched and the same file can
//! be compiled into a test elsewhere to compare it with the Python template over
//! the same bars (`mcp-server`'s `features_f2_reconciliation`, which includes
//! these files by path for exactly that reason).
//!
//! **Slice order.** Window slices are *newest first*, the order
//! `runtime::Series` already uses (`s.close[b]` is `b` bars before the decision
//! bar), because the parity contract with the legacy families is a sum order.
//! The two path-ordered measures — [`hurst`] and [`fracdiff`] — document their
//! own order at the function.
//!
//! # Registration: the lines that make these reachable
//!
//! This directory is the whole of the new arithmetic. Making a family
//! *resolvable* takes three edits outside it, in files this track does not
//! own, and they are written out here so the owner of those files can apply them
//! without reading anything else:
//!
//! 1. `crates/features/src/lib.rs`, with the other `pub mod` lines:
//!    ```text
//!    pub mod families;
//!    ```
//! 2. `crates/features/src/registry.rs`, in `FAMILIES`, after the
//!    `efficiency_ratio` row:
//!    ```text
//!    Family::new("yang_zhang_vol", p![Param::new("n", 3, 100_000)], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F2, "Yang-Zhang volatility over n bars; handles gaps between bars as well as the range"),
//!    Family::new("hurst", p![Param::new("n", 257, 100_000)], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F2, "rescaled-range Hurst exponent of bar returns; above 0.55 trending, below 0.45 mean-reverting"),
//!    Family::new("fracdiff", p![Param::new("n", 2, 100_000), Param::new("d", 1, 99)], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F2, "fractional difference of log close, order d/100 over n bars; stationary but keeps memory"),
//!    Family::new("lookback_ensemble_zscore", p![Param::new("n1", 2, 100_000), Param::new("n2", 2, 100_000), Param::new("n3", 2, 100_000)], Lookback::MaxScaled { mult: 1, plus: 0 }, CostClass::Rolling, Phase::F2, "equal-weight mean of zscore over three lookbacks; a mean-reversion rule that does not hinge on one window"),
//!    Family::new("lookback_ensemble_returns", p![Param::new("n1", 1, 100_000), Param::new("n2", 1, 100_000), Param::new("n3", 1, 100_000)], Lookback::MaxScaled { mult: 1, plus: 1 }, CostClass::Rolling, Phase::F2, "equal-weight mean of simple returns over three lookbacks; a momentum rule that does not hinge on one window"),
//!    ```
//!    `registry::EXAMPLES` already carries `n`; add `("n1", 10), ("n2", 20),
//!    ("n3", 40), ("d", 40)` so `example()` gives a paste-able name.
//! 3. `crates/features/src/runtime.rs`: a `Kind` variant per family and these
//!    arms in `Windowed::compute` and in `construct`:
//!    ```text
//!    Kind::YangZhang   => families::yang_zhang::yang_zhang_vol(&s.open, &s.high, &s.low, &s.close),
//!    Kind::Hurst       => families::hurst::hurst_rs(&log_returns(&closes_oldest_first)),
//!    Kind::Fracdiff(d) => families::fracdiff::fracdiff_log_close(&s.close, d as f64 / 100.0),
//!    Kind::EnsembleZscore { n1, n2, n3 }  => families::ensemble::mean_zscore(&s.close, &[n1, n2, n3]),
//!    Kind::EnsembleReturns { n1, n2, n3 } => families::ensemble::mean_return(&s.close, &[n1, n2, n3]),
//!
//!    "yang_zhang_vol" => windowed(Kind::YangZhang),
//!    "hurst" => windowed(Kind::Hurst),
//!    "fracdiff" => windowed(Kind::Fracdiff(p[1])),
//!    "lookback_ensemble_zscore" => windowed(Kind::EnsembleZscore { n1: p[0], n2: p[1], n3: p[2] }),
//!    "lookback_ensemble_returns" => windowed(Kind::EnsembleReturns { n1: p[0], n2: p[1], n3: p[2] }),
//!    ```
//!    `hurst`'s window is `n` bars, hence `n − 1` returns; three block sizes (16, 32, 64) need 256 returns, so its minimum `n` is
//!    257. `the_catalogue_and_the_constructor_agree_in_both_directions`
//!    fails the build if a row and an arm disagree, which is the guard that the
//!    two halves of this registration are both done.
//!
//! # Why `lookback_ensemble` is two families, and not one
//!
//! DATA-006 writes `lookback_ensemble(f, n=[…])`: the mean of *any* feature `f`
//! over several lookbacks. The registry's naming scheme is `family_p1_p2…` over
//! **integer** parameters (so that the catalogue is writable under a constrained
//! grammar by a 7B), and a feature cannot take another feature as a parameter —
//! the same limit that keeps `pct_rank(rv(close, 20), 200)` out of the language.
//! So the ensemble is registered for the two bases a strategy actually
//! ensembles, `zscore` (mean reversion) and simple returns (momentum), each over
//! exactly three lookbacks. Recorded as ADR-S5-09 in the phase file, with the
//! alternative: a generic ensemble built on a feature-of-feature mechanism, which
//! is the scheduled language item (AGENT-011 §4's ADR-D-14 amendment) and not
//! something this track should invent in the registry.

pub mod ensemble;
pub mod fracdiff;
pub mod hurst;
pub mod yang_zhang;
