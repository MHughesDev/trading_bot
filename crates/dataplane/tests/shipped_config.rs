//! The shipped `config/data_qc.toml` must parse (P-2.1).
//!
//! A unit test proves the parser works on a literal written in the test. Only
//! this proves it works on the file the platform actually loads — and
//! `deny_unknown_fields` means a typo in that file is a hard error, which you
//! want to discover in CI rather than at startup.

use dataplane::qc::QcConfig;

fn shipped() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .join("config/data_qc.toml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_shipped_config_parses() {
    let cfg = QcConfig::from_toml(&shipped()).expect("config/data_qc.toml must parse");
    assert_eq!(cfg.version().len(), 16);
}

/// The file carries the spec's numbers. If someone edits it, this fails and
/// they have to mean it.
#[test]
fn the_shipped_config_matches_the_specs_boundaries() {
    let cfg = QcConfig::from_toml(&shipped()).unwrap();
    assert!(
        (cfg.coverage.b_max_missing - 0.005).abs() < f64::EPSILON,
        "B is 0.5%"
    );
    assert!(
        (cfg.coverage.d_min_missing - 0.05).abs() < f64::EPSILON,
        "D is 5%"
    );
    assert!((cfg.bad_ticks.sigma_multiple - 8.0).abs() < f64::EPSILON);
    assert!((cfg.bad_ticks.reversal_fraction - 0.8).abs() < f64::EPSILON);
    assert!((cfg.volume.spike_multiple - 20.0).abs() < f64::EPSILON);
    // ADR-P1-7a / 7b: the two decisions the 2026-10-04 re-grade rests on.
    assert!((cfg.bad_ticks.volume_confirm_multiple - 3.0).abs() < f64::EPSILON);
    assert!((cfg.volume.max_spike_share - 0.01).abs() < f64::EPSILON);
}

/// The shipped file and the compiled defaults must name the same rules, or a
/// grade's `config_version` depends on whether the file was found.
#[test]
fn the_shipped_config_and_the_compiled_defaults_share_one_version() {
    let shipped = QcConfig::from_toml(&shipped()).unwrap();
    assert_eq!(shipped.version(), QcConfig::default().version());
}

/// Every timeframe the platform collects needs its own stale-run threshold, or
/// it silently falls back to `default` and a daily series is graded by a rule
/// written for minutes.
#[test]
fn every_collected_timeframe_has_its_own_stale_threshold() {
    let cfg = QcConfig::from_toml(&shipped()).unwrap();
    for tf in ["1m", "5m", "15m", "1h", "1d"] {
        assert!(
            cfg.stale.runs.contains_key(tf),
            "{tf} has no stale-run threshold and would fall back to `default`"
        );
    }
    assert!(
        cfg.stale.runs.contains_key("default"),
        "the fallback must exist"
    );
}

/// The shipped file and the compiled defaults must agree, or a deployment that
/// loses the file grades differently from one that has it — silently, because
/// both produce a version string and neither says which it used.
#[test]
fn the_shipped_config_is_the_compiled_default() {
    assert_eq!(
        QcConfig::from_toml(&shipped()).unwrap().version(),
        QcConfig::default().version(),
        "config/data_qc.toml has drifted from QcConfig::default(); change both or neither"
    );
}
