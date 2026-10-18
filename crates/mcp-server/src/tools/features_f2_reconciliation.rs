//! The F2 families the templates and the engine both compute, over the same
//! bars (Set S S-5.14; ADR-0029; AGENT-011 §2 F2, AC-1).
//!
//! The acceptance: *"the analysis templates and the strategy engine compute
//! identical values for the same family on the same window."*
//! `tests/volatility_reconciliation.rs` did that for the seven families that
//! existed on both sides on 2026-09-16 and recorded what was left: `yang_zhang`
//! and `hurst` were in the templates only, `efficiency_ratio` in the engine only,
//! and `fracdiff` and `lookback_ensemble` in neither. This is the second half.
//!
//! | family | engine side | template side |
//! |---|---|---|
//! | `yang_zhang_vol` | `families::yang_zhang` | `estimate_volatility._range_estimators` |
//! | `hurst` | `families::hurst` | `detect_regime._hurst` |
//! | `efficiency_ratio` | `runtime`'s registered family | `detect_regime._efficiency_ratio` (new) |
//! | `fracdiff` | `families::fracdiff` | `stationarity._fracdiff_last` (new) |
//! | `lookback_ensemble_*` | `families::ensemble` | none — compared with the registered `zscore_n` / `returns_n` it is built from |
//!
//! # Why the engine files are included by path
//!
//! The four new engine functions live in `crates/features/src/families/` and are
//! pure functions of slices, with no `crate::` reference. Registering them as
//! resolvable families needs lines in `registry.rs`, `runtime.rs` and `lib.rs`
//! that this track does not own (the exact lines are in `families/mod.rs`), so
//! until those land the functions are not reachable through
//! `features::runtime::feature`. Including the files here compiles the very same
//! source — not a copy — into this test, so the cross-language comparison runs
//! today, and the day the registration lands the same arithmetic is what the
//! registry serves. `the_catalogue_and_the_constructor_agree_in_both_directions`
//! in `features` is the guard that the registration is complete once it does.
//!
//! # The Python half
//!
//! `apps/skill-runner/reconcile_f2.py` runs the templates' own functions the way
//! the runner does (preamble prepended, then executed) and prints their answers.
//! It reimplements nothing. The bars are generated here, deterministically, with
//! regimes and gaps so every estimator has something to measure.

#[path = "../../../features/src/families/mod.rs"]
mod families;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use features::FeatureRow;
use serde_json::{json, Value};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> sits two below the root")
        .to_path_buf()
}

fn python() -> String {
    std::env::var("SKILL_RUNNER_PYTHON").unwrap_or_else(|_| "python".into())
}

struct Bars {
    open: Vec<f64>,
    high: Vec<f64>,
    low: Vec<f64>,
    close: Vec<f64>,
    volume: Vec<f64>,
}

impl Bars {
    fn rows(&self) -> Vec<FeatureRow> {
        (0..self.close.len())
            .map(|i| FeatureRow {
                ts_ns: i as i64 * 3_600_000_000_000,
                open: self.open[i],
                high: self.high[i],
                low: self.low[i],
                close: self.close[i],
                volume: self.volume[i],
            })
            .collect()
    }

    /// A series newest first, the engine's window order.
    fn newest_first(v: &[f64]) -> Vec<f64> {
        v.iter().rev().copied().collect()
    }
}

/// Regime-switching bars with overnight gaps, deterministic.
fn bars(n: usize) -> Bars {
    let mut s = 0x9E37_79B9_7F4A_7C15_u64;
    let mut unit = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f64 / (1u64 << 53) as f64
    };
    let normal = |u: &mut dyn FnMut() -> f64| (0..12).map(|_| u()).sum::<f64>() - 6.0;
    let mut b = Bars {
        open: vec![],
        high: vec![],
        low: vec![],
        close: vec![],
        volume: vec![],
    };
    let mut price = 20_000.0_f64;
    for i in 0..n {
        let regime = (i / 200) % 3;
        let drift = [0.0008, 0.0, -0.0006][regime];
        let sigma = [0.004, 0.007, 0.005][regime];
        let gap = 0.0015 * normal(&mut unit);
        let open = price * (1.0 + gap);
        let close = open * (1.0 + drift + sigma * normal(&mut unit));
        let high = open.max(close) * (1.0 + 0.003 * unit());
        let low = open.min(close) * (1.0 - 0.003 * unit());
        b.open.push(open);
        b.high.push(high);
        b.low.push(low);
        b.close.push(close);
        b.volume.push(100.0 + 900.0 * unit());
        price = close;
    }
    b
}

/// What the templates' own functions say about `bars`.
fn template_side(b: &Bars, orders: &[f64], window: usize) -> Value {
    let script = workspace_root().join("apps/skill-runner/reconcile_f2.py");
    let mut child = Command::new(python())
        .arg(&script)
        .arg(
            orders
                .iter()
                .map(f64::to_string)
                .collect::<Vec<_>>()
                .join(","),
        )
        .arg(window.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| {
            panic!(
                "could not start `{}`: {e}. The templates are Python: install \
                 apps/skill-runner/requirements.txt or set SKILL_RUNNER_PYTHON",
                python()
            )
        });
    let body = json!({
        "open": b.open, "high": b.high, "low": b.low, "close": b.close, "volume": b.volume,
    });
    {
        use std::io::Write as _;
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(body.to_string().as_bytes())
            .expect("write the bars");
    }
    let out = child.wait_with_output().expect("the script ran");
    assert!(
        out.status.success(),
        "reconcile_f2.py failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("the script printed JSON")
}

/// Same formula over the same `f64` inputs in two languages: the only legitimate
/// difference is summation order and `polyfit` against normal equations.
const TOL: f64 = 1e-9;

fn assert_close(what: &str, engine: f64, template: f64) {
    let rel = (engine - template).abs() / template.abs().max(f64::MIN_POSITIVE);
    assert!(
        rel < TOL,
        "{what}: the engine says {engine} and the template says {template} — a relative \
         difference of {rel:.3e}. These are two implementations of one estimator in two \
         languages and this test is the only thing comparing them."
    );
}

#[test]
fn yang_zhang_agrees_across_the_language_barrier() {
    let b = bars(1_500);
    let t = template_side(&b, &[0.4], 64);
    let sd = families::yang_zhang::yang_zhang_vol(
        &Bars::newest_first(&b.open),
        &Bars::newest_first(&b.high),
        &Bars::newest_first(&b.low),
        &Bars::newest_first(&b.close),
    );
    let template_variance = t["yang_zhang_variance"].as_f64().expect("a variance");
    assert!(
        template_variance > 0.0,
        "the estimator is not being exercised"
    );
    // The engine reports a per-bar deviation, the template a variance.
    assert_close("yang_zhang_vol", sd * sd, template_variance);
}

#[test]
fn hurst_agrees_across_the_language_barrier() {
    let b = bars(1_500);
    let t = template_side(&b, &[0.4], 64);
    // Increments, oldest first — the template's `np.diff(np.log(close))`.
    let r: Vec<f64> = b.close.windows(2).map(|w| w[1].ln() - w[0].ln()).collect();
    let h = families::hurst::hurst_rs(&r);
    let template = t["hurst"].as_f64().expect("a Hurst exponent");
    assert!(
        (0.0..1.2).contains(&template),
        "{template} is not an exponent; the fixture is not exercising the estimator"
    );
    assert_close("hurst", h, template);
}

/// The one family that was in the engine and not the templates. Compared with
/// the engine's *registered* family, which is what a strategy reads.
#[test]
fn efficiency_ratio_agrees_across_the_language_barrier() {
    let b = bars(1_500);
    let t = template_side(&b, &[0.4], 64);
    let rows = b.rows();
    // `efficiency_ratio_n` declares `n + 1` bars, so `n = rows - 1` spends every
    // row and is the template's whole-sample figure.
    let name = format!("efficiency_ratio_{}", rows.len() - 1);
    let f = features::runtime::feature(&name)
        .unwrap_or_else(|e| panic!("the engine does not resolve `{name}`: {e}"));
    let engine = features::runtime::value_at(f.as_ref(), &rows, rows.len() - 1)
        .expect("a value over its own window");
    let template = t["efficiency_ratio"].as_f64().expect("an efficiency ratio");
    assert!((0.0..=1.0).contains(&template));
    assert_close("efficiency_ratio", engine, template);
}

#[test]
fn fracdiff_agrees_across_the_language_barrier() {
    let b = bars(1_500);
    let window = 64;
    let orders = [0.1, 0.3, 0.4, 0.75, 0.99];
    let t = template_side(&b, &orders, window);
    let newest_first = Bars::newest_first(&b.close);
    for d in orders {
        let engine = families::fracdiff::fracdiff_log_close(&newest_first[..window], d);
        let template = t["fracdiff"][d.to_string().as_str()]
            .as_f64()
            .unwrap_or_else(|| panic!("no template value for d={d}: {}", t["fracdiff"]));
        assert_close(&format!("fracdiff d={d}"), engine, template);
    }
}

/// The ensembles have no template. What they have is a definition — the mean of
/// the registered single-window family over several lookbacks — and the engine
/// serves those families today, so the ensemble is checked against *them*.
#[test]
fn the_ensembles_are_the_mean_of_the_registered_single_window_families() {
    let b = bars(400);
    let rows = b.rows();
    let at = |name: &str| -> f64 {
        let f = features::runtime::feature(name)
            .unwrap_or_else(|e| panic!("the engine does not resolve `{name}`: {e}"));
        features::runtime::value_at(f.as_ref(), &rows, rows.len() - 1)
            .unwrap_or_else(|| panic!("`{name}` had no value"))
    };
    let newest_first = Bars::newest_first(&b.close);

    let want = (at("zscore_10") + at("zscore_20") + at("zscore_40")) / 3.0;
    let got = families::ensemble::mean_zscore(&newest_first, &[10, 20, 40]);
    assert!(
        (got - want).abs() < 1e-12,
        "zscore ensemble {got} vs {want}"
    );

    let want = (at("returns_1") + at("returns_5") + at("returns_20")) / 3.0;
    let got = families::ensemble::mean_return(&newest_first, &[1, 5, 20]);
    assert!(
        (got - want).abs() < 1e-12,
        "returns ensemble {got} vs {want}"
    );
}

/// What each family needs to be a *registered* one, derived rather than
/// written down: the arithmetic exists in this tree, and whether the registry
/// serves it is a fact the build can state. Not an assertion — a family that is
/// registered tomorrow must not fail this file — but the message is where a
/// reader looking for why `list_features` omits them finds the answer.
#[test]
fn which_of_the_new_families_the_registry_serves_yet() {
    let served: Vec<&str> = [
        "yang_zhang_vol_20",
        "hurst_257",
        "fracdiff_64_40",
        "lookback_ensemble_zscore_10_20_40",
        "lookback_ensemble_returns_1_5_20",
    ]
    .into_iter()
    .filter(|n| features::registry::is_known(n))
    .collect();
    eprintln!(
        "registry serves {} of 5 new F2 families: {served:?} (registration lines: \
         crates/features/src/families/mod.rs)",
        served.len()
    );
}
