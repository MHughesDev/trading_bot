//! The out-of-sample regime-conditional separation test (SPEC §5.4, ADR-P3-09,
//! ADR-P3-10).
//!
//! §5.4 ends: *"Validation is out-of-sample regime-conditional performance
//! separation, or the regime model is decorative."* This module is that
//! sentence made runnable. It takes the strategies' out-of-sample return series
//! and the filtered regime labels that were in force at each observation, splits
//! performance by label, and asks whether the split is bigger than chance.
//!
//! ## What is being asked, exactly
//!
//! Not "does volatility vary" — it does, by construction, because the labels are
//! volatility terciles. The question is whether *the strategies' returns* differ
//! across those terciles. If they do not, the labels are a description of the
//! market that carries no information about what the platform earns, and a gate
//! that counts them is counting nothing.
//!
//! ## Three choices that decide whether the answer means anything
//!
//! 1. **The labels are read causally.** Each observation gets the label the
//!    platform could have known at that instant — `event_time <= t` *and*
//!    `knowledge_time <= t`. Using the label of the day an observation falls in,
//!    when that label was computed later, would be the same non-causal state
//!    inference that inflates Sharpe 2.2× in §5.4's ladder. The test would then
//!    report separation that a live strategy could not have acted on.
//!
//! 2. **The null is a rotation, not a shuffle.** Daily returns inside a
//!    volatility regime are autocorrelated and so are the labels — a regime is a
//!    run of days, not a coin flip. An i.i.d. shuffle destroys that structure and
//!    produces a null distribution far too tight, so nearly any statistic looks
//!    significant. Circularly *shifting* the label sequence against the returns
//!    preserves the autocorrelation of both exactly and destroys only their
//!    alignment, which is the thing under test (ADR-P3-10).
//!
//! 3. **Alpha is pre-registered.** [`ALPHA`] is 0.05, chosen before the first run
//!    and not moved afterwards. A separation test tuned until the regime model
//!    passes is a curve-fitted backtest wearing a lab coat, in the one place the
//!    platform is supposed to be immune to it.
//!
//! ## Where the answer lives, and why it is not reachable from a gate
//!
//! `regime_research.regime_separation`. The research schema, which
//! `backtest_role`, `agent_role` and `internal_ml_role` hold no USAGE on. That is
//! not filing: the result is a full-sample statement about which regime paid,
//! and a strategy or a gate that could read it would be reading a summary of its
//! own future. This module is the only one in the crate that names the research
//! schema, and a static test asserts the ones that evaluate do not.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::knowledge::{labels_as_of, RegimeLabel, RegimeScope, RegimeStore, RULE_MODEL_VERSION};

/// The pre-registered significance level. Chosen before the first run.
///
/// It is a constant rather than a parameter for the reason §12.7 gives about
/// every other threshold: a number the caller supplies is a number the caller
/// can raise after seeing the answer.
pub const ALPHA: f64 = 0.05;

/// Rotations drawn for the null distribution.
///
/// When a series is shorter than this, every distinct rotation is enumerated
/// instead and the test becomes exact — which also makes the floor on the
/// achievable p-value visible: with `n` observations there are `n` rotations, so
/// nothing shorter than 20 points can ever reach 0.05.
pub const PERMUTATIONS: usize = 2_000;

/// Observations a label needs before it is treated as a regime at all.
///
/// Below this the group's mean is noise, and a spread measured against noise is
/// the easiest false positive in the whole procedure.
pub const MIN_OBS_PER_REGIME: usize = 20;

/// Distinct labels needed before there is a comparison to make.
pub const MIN_REGIMES: usize = 2;

/// How far before a series' first observation the label history is read.
///
use crate::knowledge::LABEL_LOOKBEHIND_DAYS;

/// One strategy's out-of-sample series, already aligned to its causal labels.
#[derive(Clone, Debug, PartialEq)]
pub struct LabelledSeries {
    /// What produced it — a trial id, or a named reconstruction.
    pub source: String,
    /// In time order. The rotation null depends on that ordering being real.
    pub points: Vec<(RegimeLabel, f64)>,
}

/// Performance inside one regime.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegimeStats {
    pub label: String,
    pub n: usize,
    /// In per-series standard deviations (see [`separate`]).
    pub mean: f64,
    pub sd: f64,
    /// Annualized, and therefore comparable across regimes of different lengths.
    pub sharpe: f64,
}

/// What the test found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeparationResult {
    pub series_count: usize,
    pub observation_count: usize,
    pub labelled_count: usize,
    pub regimes_present: usize,
    pub per_regime: Vec<RegimeStats>,
    /// Best-minus-worst regime mean, in per-series standard deviations.
    pub spread_mean: Option<f64>,
    /// Best-minus-worst annualized Sharpe.
    pub spread_sharpe: Option<f64>,
    /// Kruskal–Wallis H and its chi-square p-value. **Reported, not decisive** —
    /// it assumes independent observations, which returns inside a volatility
    /// regime are not, so it is anti-conservative here.
    pub kruskal_h: Option<f64>,
    pub kruskal_p: Option<f64>,
    /// The decisive one: the rotation test's p-value.
    pub permutation_p: Option<f64>,
    pub permutations: usize,
    pub alpha: f64,
    /// `None` means the question could not be answered, which is never a pass.
    pub separates: Option<bool>,
    pub verdict: String,
}

impl SeparationResult {
    /// The honest empty answer.
    fn not_computable(
        series_count: usize,
        observation_count: usize,
        labelled_count: usize,
        regimes_present: usize,
        per_regime: Vec<RegimeStats>,
        why: impl Into<String>,
    ) -> Self {
        Self {
            series_count,
            observation_count,
            labelled_count,
            regimes_present,
            per_regime,
            spread_mean: None,
            spread_sharpe: None,
            kruskal_h: None,
            kruskal_p: None,
            permutation_p: None,
            permutations: 0,
            alpha: ALPHA,
            separates: None,
            verdict: why.into(),
        }
    }
}

/// Run the separation test.
///
/// **Scale.** Each series is divided by its own out-of-sample standard deviation
/// before pooling. Strategies sized differently are otherwise incomparable and
/// the loudest one decides the statistic; dividing by a per-series constant
/// cannot create or destroy separation within a series, only put the series on
/// one axis. The reported means are therefore in standard deviations, and the
/// Sharpes — being scale-free already — are unaffected.
///
/// **Statistic.** `max_label(mean) − min_label(mean)` over the labels that clear
/// [`MIN_OBS_PER_REGIME`]. One number, fixed in advance, and the same number is
/// recomputed under every rotation.
///
/// `seed` makes the rotation draw reproducible; the same inputs give the same
/// p-value, which is what lets a re-run be a check rather than a new experiment.
#[must_use]
#[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
pub fn separate(series: &[LabelledSeries], periods_per_year: f64, seed: u64) -> SeparationResult {
    let series_count = series.len();
    let observation_count: usize = series.iter().map(|s| s.points.len()).sum();

    // Standardize each series by its own sd, and drop any series that never
    // moves: a constant series contributes nothing to a spread and its
    // standardization is a division by zero.
    let mut standardized: Vec<Vec<(RegimeLabel, f64)>> = Vec::new();
    for s in series {
        let n = s.points.len();
        if n < MIN_OBS_PER_REGIME {
            continue;
        }
        let mean = s.points.iter().map(|(_, r)| r).sum::<f64>() / n as f64;
        let var = s
            .points
            .iter()
            .map(|(_, r)| (r - mean).powi(2))
            .sum::<f64>()
            / (n as f64 - 1.0);
        let sd = var.sqrt();
        if !(sd.is_finite() && sd > 0.0) {
            continue;
        }
        standardized.push(s.points.iter().map(|(l, r)| (*l, r / sd)).collect());
    }

    let labelled_count: usize = standardized.iter().map(Vec::len).sum();
    let pooled: Vec<(RegimeLabel, f64)> = standardized
        .iter()
        .flat_map(|s| s.iter().copied())
        .collect();
    let per_regime = regime_stats(&pooled, periods_per_year);
    let usable: Vec<&RegimeStats> = per_regime
        .iter()
        .filter(|s| s.n >= MIN_OBS_PER_REGIME)
        .collect();
    let regimes_present = per_regime.len();

    if usable.len() < MIN_REGIMES {
        return SeparationResult::not_computable(
            series_count,
            observation_count,
            labelled_count,
            regimes_present,
            per_regime.clone(),
            format!(
                "{} of {regimes_present} labels have the {MIN_OBS_PER_REGIME} observations a \
                 regime needs; there is no comparison to make",
                usable.len()
            ),
        );
    }

    let hi = usable
        .iter()
        .map(|s| s.mean)
        .fold(f64::NEG_INFINITY, f64::max);
    let lo = usable.iter().map(|s| s.mean).fold(f64::INFINITY, f64::min);
    let spread = hi - lo;
    let sharpe_hi = usable
        .iter()
        .map(|s| s.sharpe)
        .fold(f64::NEG_INFINITY, f64::max);
    let sharpe_lo = usable
        .iter()
        .map(|s| s.sharpe)
        .fold(f64::INFINITY, f64::min);

    let (h, kruskal_p) = kruskal_wallis(&pooled);

    // The rotation null.
    let mut rng = SplitMix64::new(seed);
    let mut at_least_as_extreme = 0usize;
    let mut draws = 0usize;
    for _ in 0..PERMUTATIONS {
        let rotated: Vec<(RegimeLabel, f64)> = standardized
            .iter()
            .flat_map(|s| {
                let n = s.len();
                let k = (rng.next_u64() % n as u64) as usize;
                (0..n).map(move |i| (s[(i + k) % n].0, s[i].1))
            })
            .collect();
        let stats = regime_stats(&rotated, periods_per_year);
        let usable: Vec<&RegimeStats> =
            stats.iter().filter(|s| s.n >= MIN_OBS_PER_REGIME).collect();
        if usable.len() < MIN_REGIMES {
            continue;
        }
        let hi = usable
            .iter()
            .map(|s| s.mean)
            .fold(f64::NEG_INFINITY, f64::max);
        let lo = usable.iter().map(|s| s.mean).fold(f64::INFINITY, f64::min);
        draws += 1;
        if hi - lo >= spread {
            at_least_as_extreme += 1;
        }
    }

    if draws == 0 {
        return SeparationResult::not_computable(
            series_count,
            observation_count,
            labelled_count,
            regimes_present,
            per_regime,
            "no rotation of the labels produced a comparable split; the label sequence is too \
             degenerate to build a null from",
        );
    }

    // The +1s are Davison–Hinkley: the observed arrangement is itself one of the
    // arrangements under the null, and omitting it can report p = 0, which no
    // finite permutation test can justify.
    let p = (at_least_as_extreme as f64 + 1.0) / (draws as f64 + 1.0);
    let separates = p < ALPHA;

    let verdict = if separates {
        format!(
            "separates: best-minus-worst regime mean {spread:.4} sd (Sharpe spread {:.2}), \
             rotation p = {p:.4} over {draws} rotations, under the pre-registered {ALPHA}",
            sharpe_hi - sharpe_lo
        )
    } else {
        format!(
            "does not separate: best-minus-worst regime mean {spread:.4} sd (Sharpe spread \
             {:.2}), rotation p = {p:.4} over {draws} rotations, against the pre-registered \
             {ALPHA}. By SPEC §5.4's own standard this regime model is decorative on this \
             evidence",
            sharpe_hi - sharpe_lo
        )
    };

    SeparationResult {
        series_count,
        observation_count,
        labelled_count,
        regimes_present,
        per_regime,
        spread_mean: Some(spread),
        spread_sharpe: Some(sharpe_hi - sharpe_lo),
        kruskal_h: h,
        kruskal_p,
        permutation_p: Some(p),
        permutations: draws,
        alpha: ALPHA,
        separates: Some(separates),
        verdict,
    }
}

/// Mean, sd and annualized Sharpe per label, in label order.
#[allow(clippy::cast_precision_loss)]
fn regime_stats(points: &[(RegimeLabel, f64)], periods_per_year: f64) -> Vec<RegimeStats> {
    let mut by_label: BTreeMap<&'static str, Vec<f64>> = BTreeMap::new();
    for (l, r) in points {
        by_label.entry(l.as_str()).or_default().push(*r);
    }
    let mut out = Vec::new();
    for label in RegimeLabel::ALL {
        let Some(v) = by_label.get(label.as_str()) else {
            continue;
        };
        let n = v.len();
        let mean = v.iter().sum::<f64>() / n as f64;
        let sd = if n >= 2 {
            (v.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0)).sqrt()
        } else {
            0.0
        };
        let sharpe = if sd > 0.0 {
            mean / sd * periods_per_year.sqrt()
        } else {
            0.0
        };
        out.push(RegimeStats {
            label: label.as_str().to_string(),
            n,
            mean,
            sd,
            sharpe,
        });
    }
    out
}

/// Kruskal–Wallis H with the standard tie correction, and its chi-square
/// p-value.
///
/// Reported for readers who expect it, and deliberately not the decision. It
/// assumes the observations are independent; consecutive days inside one
/// volatility regime are not, so H's reference distribution is wrong in the
/// direction that finds significance.
#[allow(clippy::cast_precision_loss)]
fn kruskal_wallis(points: &[(RegimeLabel, f64)]) -> (Option<f64>, Option<f64>) {
    let n = points.len();
    if n < 3 {
        return (None, None);
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| {
        points[*a]
            .1
            .partial_cmp(&points[*b].1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    // Mid-ranks for ties, and the tie sum the correction needs.
    let mut rank = vec![0.0_f64; n];
    let mut ties = 0.0_f64;
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && (points[order[j + 1]].1 - points[order[i]].1).abs() <= f64::EPSILON {
            j += 1;
        }
        let t = (j - i + 1) as f64;
        let mid = (i as f64 + j as f64) / 2.0 + 1.0;
        for k in i..=j {
            rank[order[k]] = mid;
        }
        if t > 1.0 {
            ties += t * t * t - t;
        }
        i = j + 1;
    }

    let mut groups: BTreeMap<&'static str, (f64, usize)> = BTreeMap::new();
    for (idx, (l, _)) in points.iter().enumerate() {
        let e = groups.entry(l.as_str()).or_insert((0.0, 0));
        e.0 += rank[idx];
        e.1 += 1;
    }
    let k = groups.len();
    if k < 2 {
        return (None, None);
    }
    let nf = n as f64;
    let sum: f64 = groups.values().map(|(r, c)| r * r / *c as f64).sum();
    let mut h = 12.0 / (nf * (nf + 1.0)) * sum - 3.0 * (nf + 1.0);
    let correction = 1.0 - ties / (nf * nf * nf - nf);
    if correction > 0.0 {
        h /= correction;
    }
    if !h.is_finite() || h < 0.0 {
        return (None, None);
    }
    (Some(h), chi2_sf(h, k - 1))
}

/// `P(X > x)` for a chi-square with `df` degrees of freedom.
///
/// Closed form for df 1 and 2, which is every case this module can produce —
/// the rule tier has three labels, so `df = k - 1 <= 2`. A larger label set
/// returns `None` rather than an approximation nobody checked.
fn chi2_sf(x: f64, df: usize) -> Option<f64> {
    match df {
        1 => Some(erfc((x / 2.0).sqrt())),
        2 => Some((-x / 2.0).exp()),
        _ => None,
    }
}

/// Abramowitz & Stegun 7.1.26, good to ~1.5e-7 — far finer than any decision
/// made on it here.
fn erfc(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let y = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))))
        * (-x * x).exp();
    if x >= 0.0 {
        y
    } else {
        2.0 - y
    }
}

/// SplitMix64. A named, reproducible stream: the same inputs must give the same
/// p-value or a re-run is a new experiment rather than a check.
struct SplitMix64(u64);

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// The job
// ───────────────────────────────────────────────────────────────────────────────

/// One stored run of the test.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SeparationRun {
    pub market_scope: String,
    pub model_version: String,
    pub input_digest: String,
    /// False when this exact question had already been answered — the job is a
    /// timer, and a timer that rewrites the same answer daily makes the history
    /// useless.
    pub written: bool,
    pub result: SeparationResult,
}

/// Runs the separation test and records it in `regime_research`.
pub struct SeparationJob {
    pg: PgPool,
}

impl SeparationJob {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    /// Run the test for one scope against the tenant's out-of-sample series.
    ///
    /// Re-runs are keyed on `(scope, model_version, input_digest)`: a new regime
    /// model, or new labels, or a new trial series, is a new question and gets a
    /// new row. The same inputs write nothing.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn run(
        &self,
        tenant_id: &str,
        scope: &RegimeScope,
        model_version: &str,
    ) -> Result<SeparationRun, sqlx::Error> {
        let series = self.ledger_series(tenant_id).await?;
        self.run_on(tenant_id, scope, model_version, &series).await
    }

    /// The same run, over series supplied by the caller.
    ///
    /// Split out so the measurement can be driven from series the ledger does
    /// not hold yet without a second copy of the alignment, the statistic or the
    /// write. There is no "skip the record" path: every run this method performs
    /// is offered to the table under its own digest.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn run_on(
        &self,
        tenant_id: &str,
        scope: &RegimeScope,
        model_version: &str,
        series: &[(String, Vec<(DateTime<Utc>, f64)>)],
    ) -> Result<SeparationRun, sqlx::Error> {
        let store = RegimeStore::new(self.pg.clone());
        let (from, to) = span(series);
        let history = store
            .history(
                &scope.market_scope,
                model_version,
                // Back far enough to catch the label already in force when the
                // first observation lands. A daily label is stamped at an
                // earlier bar's close, and reading from the first observation
                // would leave the start of every series unlabelled — which the
                // callers correctly refuse to work around.
                from - chrono::Duration::days(LABEL_LOOKBEHIND_DAYS),
                to,
            )
            .await?;

        let mut labelled = Vec::new();
        let mut spacings: Vec<i64> = Vec::new();
        for (source, points) in series {
            let times: Vec<DateTime<Utc>> = points.iter().map(|(t, _)| *t).collect();
            let labels = labels_as_of(&history, &times);
            let aligned: Vec<(RegimeLabel, f64)> = labels
                .iter()
                .zip(points)
                .filter_map(|(l, (_, r))| l.map(|l| (l, *r)))
                .collect();
            if aligned.is_empty() {
                continue;
            }
            for w in times.windows(2) {
                spacings.push((w[1] - w[0]).num_seconds().max(1));
            }
            labelled.push(LabelledSeries {
                source: source.clone(),
                points: aligned,
            });
        }

        let periods_per_year = periods_per_year(&spacings);
        let digest = input_digest(&history, series);
        // The seed is the digest, so the null draw is a function of the inputs:
        // the same question gets the same answer, and nobody can re-roll a
        // p-value by re-running the job.
        let seed = seed_from(&digest);
        let result = separate(&labelled, periods_per_year, seed);

        let written = sqlx::query(
            "INSERT INTO regime_research.regime_separation
                 (separation_id, tenant_id, market_scope, model_version, input_digest,
                  series_count, observation_count, labelled_count, regimes_present,
                  per_regime, spread_mean, spread_sharpe, kruskal_h, kruskal_p,
                  permutation_p, block_length, permutations, alpha, separates, verdict)
             VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                     $13, $14, NULL, $15, $16, $17, $18)
             ON CONFLICT ON CONSTRAINT uq_regime_separation_inputs DO NOTHING",
        )
        .bind(tenant_id)
        .bind(&scope.market_scope)
        .bind(model_version)
        .bind(&digest)
        .bind(i32::try_from(result.series_count).unwrap_or(i32::MAX))
        .bind(i32::try_from(result.observation_count).unwrap_or(i32::MAX))
        .bind(i32::try_from(result.labelled_count).unwrap_or(i32::MAX))
        .bind(i32::try_from(result.regimes_present).unwrap_or(i32::MAX))
        .bind(serde_json::to_value(&result.per_regime).unwrap_or(serde_json::Value::Null))
        .bind(result.spread_mean)
        .bind(result.spread_sharpe)
        .bind(result.kruskal_h)
        .bind(result.kruskal_p)
        .bind(result.permutation_p)
        .bind(i32::try_from(result.permutations).unwrap_or(i32::MAX))
        .bind(result.alpha)
        .bind(result.separates)
        .bind(&result.verdict)
        .execute(&self.pg)
        .await?
        .rows_affected()
            > 0;

        Ok(SeparationRun {
            market_scope: scope.market_scope.clone(),
            model_version: model_version.to_string(),
            input_digest: digest,
            written,
            result,
        })
    }

    /// Every out-of-sample return series the ledger holds for the tenant.
    ///
    /// `mlops.trial_return_series` is what §12.4 already counts N_eff over, so
    /// it is the platform's own answer to "which series are out of sample" — and
    /// using a second answer here would let the separation test measure
    /// something the gates never saw.
    async fn ledger_series(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<(String, Vec<(DateTime<Utc>, f64)>)>, sqlx::Error> {
        // Through the tenant transaction, not the bare pool. The table has
        // FORCE ROW LEVEL SECURITY and its policy reads `app.tenant_id`, so a
        // plain read returns **zero rows rather than an error** — which would
        // make "the ledger has no series" indistinguishable from "this job
        // forgot to say who it is".
        let mut tx = ledger::pg::tenant_tx(&self.pg, tenant_id)
            .await
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        let rows: Vec<(uuid::Uuid, Vec<DateTime<Utc>>, Vec<f64>)> = sqlx::query_as(
            "SELECT trial_id, ts, returns FROM mlops.trial_return_series
              WHERE tenant_id = $1 ORDER BY recorded_at",
        )
        .bind(tenant_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .map(|(id, ts, returns)| (id.to_string(), ts.into_iter().zip(returns).collect()))
            .collect())
    }
}

/// The widest span the series cover, so one history read serves all of them.
fn span(series: &[(String, Vec<(DateTime<Utc>, f64)>)]) -> (DateTime<Utc>, DateTime<Utc>) {
    let mut from = DateTime::<Utc>::MAX_UTC;
    let mut to = DateTime::<Utc>::MIN_UTC;
    for (_, points) in series {
        for (t, _) in points {
            from = from.min(*t);
            to = to.max(*t);
        }
    }
    if from > to {
        let now = Utc::now();
        return (now, now);
    }
    (from, to)
}

/// Observations per year, from the median observed spacing.
///
/// Measured rather than assumed, for ADR-P2-32's reason: the same per-step
/// Sharpe on hourly and daily data is a wildly different annual number, and a
/// fixed 252 inflates one of them.
#[allow(clippy::cast_precision_loss)]
fn periods_per_year(spacings: &[i64]) -> f64 {
    if spacings.is_empty() {
        return 252.0;
    }
    let mut s = spacings.to_vec();
    s.sort_unstable();
    let median = s[s.len() / 2].max(1) as f64;
    (365.25 * 86_400.0) / median
}

/// The canonical digest of everything that went into a run.
///
/// Built from three [`ledger::neff::ReturnSeries`] digests — the labels, the
/// times those labels became knowable, and the returns — rather than a fourth
/// hashing routine. CLAUDE.md §4 asks for one hash function everywhere, and the
/// series digest is the one the ledger already chains trials on.
///
/// The knowledge times are in the digest for a reason that is not bookkeeping: a
/// relabelled history with the same labels but earlier knowledge times is a
/// different, more permissive question, and it must not silently reuse the
/// previous answer.
fn input_digest(
    history: &[crate::knowledge::RegimeObservation],
    series: &[(String, Vec<(DateTime<Utc>, f64)>)],
) -> String {
    use ledger::neff::ReturnSeries;

    let event_times: Vec<DateTime<Utc>> = history.iter().map(|o| o.event_time).collect();
    let labels = ReturnSeries {
        timestamps: event_times.clone(),
        returns: history
            .iter()
            .map(|o| {
                RegimeLabel::ALL
                    .iter()
                    .position(|l| *l == o.label)
                    .unwrap_or(usize::MAX) as f64
            })
            .collect(),
    };
    let knowledge = ReturnSeries {
        timestamps: event_times,
        #[allow(clippy::cast_precision_loss)]
        returns: history
            .iter()
            .map(|o| o.knowledge_time.timestamp() as f64)
            .collect(),
    };
    let flat = ReturnSeries {
        timestamps: series
            .iter()
            .flat_map(|(_, p)| p.iter().map(|(t, _)| *t))
            .collect(),
        returns: series
            .iter()
            .flat_map(|(_, p)| p.iter().map(|(_, r)| *r))
            .collect(),
    };
    format!(
        "{}|{}|{}",
        labels.digest(),
        knowledge.digest(),
        flat.digest()
    )
}

/// A reproducible seed from the digest's own bytes.
fn seed_from(digest: &str) -> u64 {
    let b = digest.as_bytes();
    let mut out = [0_u8; 8];
    for (i, slot) in out.iter_mut().enumerate() {
        // Skip the `sha256:` prefix the canonical digest carries.
        *slot = b.get(i + 7).copied().unwrap_or(0);
    }
    u64::from_le_bytes(out)
}

/// The model version the rule tier writes, re-exported so a caller does not have
/// to know which module names it.
#[must_use]
pub const fn rule_model_version() -> &'static str {
    RULE_MODEL_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(name: &str, points: Vec<(RegimeLabel, f64)>) -> LabelledSeries {
        LabelledSeries {
            source: name.to_string(),
            points,
        }
    }

    /// Runs of low and high vol of irregular length — as real regimes are —
    /// with a real difference in what is earned inside them. The test must find
    /// it: a separation test that cannot detect separation is worth nothing as
    /// evidence of its absence.
    #[test]
    fn a_real_separation_is_found() {
        let mut points = Vec::new();
        let mut rng = SplitMix64::new(7);
        let mut label = RegimeLabel::LowVol;
        while points.len() < 800 {
            let run = 8 + (rng.next_u64() % 40) as usize;
            let drift = if label == RegimeLabel::LowVol {
                1.0
            } else {
                -1.0
            };
            for _ in 0..run {
                // Deterministic pseudo-noise in [-0.5, 0.5].
                let noise = (rng.next_u64() % 1000) as f64 / 1000.0 - 0.5;
                points.push((label, drift + noise));
            }
            label = if label == RegimeLabel::LowVol {
                RegimeLabel::HighVol
            } else {
                RegimeLabel::LowVol
            };
        }
        let out = separate(&[series("real", points)], 252.0, 1);
        assert_eq!(out.separates, Some(true), "{}", out.verdict);
        assert!(out.permutation_p.unwrap() < ALPHA);
        assert!(out.spread_mean.unwrap() > 0.5);
    }

    /// A label sequence that is exactly periodic cannot be told apart from its
    /// own rotations, and the test reports that rather than a small p-value.
    ///
    /// This is a real limit, kept as a test so nobody rediscovers it as a bug:
    /// with regimes alternating every 20 observations, one rotation in twenty
    /// realigns them perfectly and another anti-aligns them, and the statistic —
    /// a spread, which is unsigned — cannot tell those apart. Real volatility
    /// terciles are not periodic; a synthetic fixture that is, is the fixture's
    /// problem. The honest answer on such a sequence is "cannot distinguish",
    /// and lowering the bar to force a pass here is exactly the tuning §5.4
    /// warns about.
    #[test]
    fn a_perfectly_periodic_label_sequence_cannot_be_separated_from_its_rotations() {
        let mut points = Vec::new();
        let mut rng = SplitMix64::new(7);
        for block in 0..30 {
            let (label, drift) = if block % 2 == 0 {
                (RegimeLabel::LowVol, 1.0)
            } else {
                (RegimeLabel::HighVol, -1.0)
            };
            for _ in 0..20 {
                let noise = (rng.next_u64() % 1000) as f64 / 1000.0 - 0.5;
                points.push((label, drift + noise));
            }
        }
        let out = separate(&[series("periodic", points)], 252.0, 1);
        // The effect is enormous and the p-value is still not significant.
        assert!(out.spread_mean.unwrap() > 1.5);
        assert_eq!(out.separates, Some(false), "{}", out.verdict);
    }

    /// The same label structure, and returns that owe nothing to it. The test
    /// must NOT find separation — and this is the case that matters, because it
    /// is the one a tuned test would quietly start passing.
    #[test]
    fn noise_does_not_separate() {
        let mut points = Vec::new();
        let mut rng = SplitMix64::new(11);
        for block in 0..30 {
            let label = if block % 2 == 0 {
                RegimeLabel::LowVol
            } else {
                RegimeLabel::HighVol
            };
            for _ in 0..20 {
                let noise = (rng.next_u64() % 1000) as f64 / 1000.0 - 0.5;
                points.push((label, noise));
            }
        }
        let out = separate(&[series("noise", points)], 252.0, 2);
        assert_eq!(out.separates, Some(false), "{}", out.verdict);
        assert!(out.verdict.contains("decorative"), "{}", out.verdict);
    }

    /// An i.i.d. shuffle would call this significant; the rotation null does
    /// not. The difference is the whole of ADR-P3-10: labels and returns that
    /// are both autocorrelated will line up by accident far more often than an
    /// independence null admits.
    #[test]
    fn autocorrelated_noise_is_not_significance() {
        // One slow sine in the returns, one slow square wave in the labels, at
        // nearby periods. Nothing causal connects them.
        let n = 600;
        let points: Vec<(RegimeLabel, f64)> = (0..n)
            .map(|i| {
                let x = i as f64;
                let label = if (x / 47.0).sin() > 0.0 {
                    RegimeLabel::HighVol
                } else {
                    RegimeLabel::LowVol
                };
                (label, (x / 53.0).sin())
            })
            .collect();
        let out = separate(&[series("wave", points)], 252.0, 3);
        assert_eq!(
            out.separates,
            Some(false),
            "a rotation null must not call two unrelated slow oscillations a regime effect: {}",
            out.verdict
        );
    }

    /// Too few observations is "not computable", which is a third state and is
    /// never a pass.
    #[test]
    fn a_sample_too_short_answers_nothing() {
        let points = vec![(RegimeLabel::LowVol, 0.1), (RegimeLabel::HighVol, -0.1)];
        let out = separate(&[series("tiny", points)], 252.0, 4);
        assert_eq!(out.separates, None);
        assert!(out.permutation_p.is_none());
        assert!(out.verdict.contains("no comparison"), "{}", out.verdict);
    }

    /// One regime, however many observations, is not a comparison.
    #[test]
    fn one_regime_is_not_a_comparison() {
        let points: Vec<(RegimeLabel, f64)> = (0..200)
            .map(|i| (RegimeLabel::MidVol, if i % 2 == 0 { 0.1 } else { -0.1 }))
            .collect();
        let out = separate(&[series("flat", points)], 252.0, 5);
        assert_eq!(out.separates, None, "{}", out.verdict);
    }

    /// The p-value is a function of the inputs, not of when the job ran.
    #[test]
    fn the_same_inputs_give_the_same_p_value() {
        let points: Vec<(RegimeLabel, f64)> = (0..300)
            .map(|i| {
                let label = if (i / 25) % 2 == 0 {
                    RegimeLabel::LowVol
                } else {
                    RegimeLabel::HighVol
                };
                (label, ((i * 37) % 13) as f64 / 13.0 - 0.5)
            })
            .collect();
        let a = separate(&[series("s", points.clone())], 252.0, 99);
        let b = separate(&[series("s", points)], 252.0, 99);
        assert_eq!(a.permutation_p, b.permutation_p);
    }

    /// df = 2 is exact, and the two closed forms agree with the values every
    /// table gives.
    #[test]
    fn the_chi_square_tail_is_the_one_in_the_table() {
        assert!((chi2_sf(5.991, 2).unwrap() - 0.05).abs() < 1e-3);
        assert!((chi2_sf(3.841, 1).unwrap() - 0.05).abs() < 1e-3);
        assert_eq!(chi2_sf(1.0, 5), None, "no approximation nobody checked");
    }

    /// Kruskal–Wallis on two clearly different groups is large; on one pooled
    /// population it is near zero.
    #[test]
    fn kruskal_wallis_behaves() {
        let apart: Vec<(RegimeLabel, f64)> = (0..60)
            .map(|i| {
                if i < 30 {
                    (RegimeLabel::LowVol, i as f64)
                } else {
                    (RegimeLabel::HighVol, 1000.0 + i as f64)
                }
            })
            .collect();
        let (h, p) = kruskal_wallis(&apart);
        assert!(h.unwrap() > 30.0);
        assert!(p.unwrap() < 0.001);

        let mixed: Vec<(RegimeLabel, f64)> = (0..60)
            .map(|i| {
                let l = if i % 2 == 0 {
                    RegimeLabel::LowVol
                } else {
                    RegimeLabel::HighVol
                };
                (l, i as f64)
            })
            .collect();
        let (h, p) = kruskal_wallis(&mixed);
        assert!(h.unwrap() < 1.0);
        assert!(p.unwrap() > 0.3);
    }

    /// Annualization comes from the observed spacing, never from an assumed 252.
    #[test]
    fn the_annualization_is_measured() {
        let daily = periods_per_year(&[86_400, 86_400, 86_400]);
        assert!((daily - 365.25).abs() < 0.01);
        let hourly = periods_per_year(&[3_600, 3_600]);
        assert!((hourly - 365.25 * 24.0).abs() < 0.01);
        assert!((periods_per_year(&[]) - 252.0).abs() < f64::EPSILON);
    }

    /// The pre-registered alpha is a constant, and no function here takes one.
    /// If this test has to be edited to make a run pass, that edit is the
    /// finding — a separation test tuned until the regime model looks good is a
    /// curve-fitted backtest in the one place the platform is supposed to be
    /// immune to it.
    #[test]
    fn alpha_is_pre_registered() {
        assert!((ALPHA - 0.05).abs() < f64::EPSILON);
        let src = include_str!("regime_separation.rs");
        let production = &src[..src.find("#[cfg(test)]").expect("a test module")];
        // Every function signature in the module, checked for a way in.
        for sig in production.split("fn ").skip(1) {
            let params = sig.split(')').next().unwrap_or_default();
            assert!(
                !params.contains("alpha"),
                "alpha must not be a parameter a caller can raise after seeing the answer: fn {}",
                sig.lines().next().unwrap_or_default()
            );
        }
    }
}
