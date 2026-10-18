//! Quality tests for analysis-code skills against a real Python (Set R-a RA-3.5).
//!
//! ```text
//! cargo test -p skills --test quality_live -- --ignored --test-threads=1
//! ```

use skills::quality::{default_cuts, noise_null, truncation_check, white_noise, SeriesSpec};
use skills::runner::RunnerConfig;
use skills::{Contract, Skill};

fn config() -> RunnerConfig {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .to_path_buf();
    RunnerConfig::from_workspace(&root)
}

fn skill(source: &str, outputs: &[&str]) -> Skill {
    Skill::author(
        "series_skill",
        "A series skill authored by the quality tests, described at length.",
        source,
        Contract::new(["ts", "values"], outputs.to_vec(), "test").unwrap(),
        None,
    )
    .expect("authors")
}

fn series(n: usize) -> (Vec<i64>, Vec<f64>) {
    let ts: Vec<i64> = (0..n as i64).map(|i| 1_700_000_000 + i * 60).collect();
    let mut x = white_noise(n, 3);
    // Gaps. The runner serialises NaN as null, which the skills below read as missing.
    for i in [40usize, 41, 90, 91] {
        x[i] = f64::NAN;
    }
    (ts, x)
}

/// A causal skill: forward-fill, then an expanding mean. Never reads ahead.
const CAUSAL: &str = r#"def run(payload):
    xs = payload['values']
    out, last, total, n = [], None, 0.0, 0
    for v in xs:
        if v is not None and v == v:
            last = v
        if last is not None:
            total += last
            n += 1
            out.append(total / n)
        else:
            out.append(None)
    return {'result': out}
"#;

/// A leaky skill: it back-fills a gap with the **next** value, which is the future.
const BFILL: &str = r#"def run(payload):
    out = [None if (v is None or v != v) else v for v in payload['values']]
    nxt = None
    for i in range(len(out) - 1, -1, -1):
        if out[i] is None:
            out[i] = nxt
        else:
            nxt = out[i]
    return {'result': out}
"#;

#[tokio::test]
#[ignore = "requires python on PATH"]
async fn a_causal_skill_passes_truncation() {
    let (ts, x) = series(120);
    let r = truncation_check(&skill(CAUSAL, &["result"]), &config(), &SeriesSpec::default(), &ts, &x, &default_cuts(120, 4))
        .await
        .unwrap();
    assert!(r.passed, "{r:?}");
    assert_eq!(r.cuts_checked, 4);
}

#[tokio::test]
#[ignore = "requires python on PATH"]
async fn a_bfill_skill_fails_truncation_naming_the_first_differing_timestamp() {
    let (ts, x) = series(120);
    // Cut at row 40, the first gap: the prefix has no later value to fill it from, so the
    // truncated run answers `None` where the full run answered the value from row 42.
    let r = truncation_check(&skill(BFILL, &["result"]), &config(), &SeriesSpec::default(), &ts, &x, &[40, 60, 100])
        .await
        .unwrap();
    assert!(!r.passed, "a back-fill reads the future and must fail: {r:?}");
    assert_eq!(r.first_differing_ts, Some(ts[40]), "{r:?}");
    assert_eq!(r.cut_ts, Some(ts[40]));
    assert!(r.full.is_some() && r.truncated.is_none());
}

/// A calibrated test: the mean is significant at 5% under the null (normal approximation).
const WELL_CALIBRATED: &str = r#"import math
def run(payload):
    xs = payload['values']
    n = len(xs)
    m = sum(xs) / n
    sd = math.sqrt(sum((v - m) ** 2 for v in xs) / (n - 1))
    z = m / (sd / math.sqrt(n))
    return {'p_value': math.erfc(abs(z) / math.sqrt(2))}
"#;

/// A broken test: its statistic is inflated, so noise looks significant far too often.
const FINDS_STRUCTURE: &str = r#"import math
def run(payload):
    xs = payload['values']
    n = len(xs)
    m = sum(xs) / n
    sd = math.sqrt(sum((v - m) ** 2 for v in xs) / (n - 1))
    z = 2.0 * m / (sd / math.sqrt(n))
    return {'p_value': math.erfc(abs(z) / math.sqrt(2))}
"#;

#[tokio::test]
#[ignore = "requires python on PATH"]
async fn a_calibrated_test_passes_the_noise_null_and_one_that_finds_structure_in_noise_fails() {
    let spec = SeriesSpec::default();
    let good = noise_null(&skill(WELL_CALIBRATED, &["p_value"]), &config(), &spec, 400, 200, 11).await.unwrap();
    assert!(good.passed, "{good:?}");
    let bad = noise_null(&skill(FINDS_STRUCTURE, &["p_value"]), &config(), &spec, 200, 200, 11).await.unwrap();
    assert!(!bad.passed, "{bad:?}");
    assert!(bad.false_positive_rate > 0.07);
}
