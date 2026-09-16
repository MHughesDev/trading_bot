//! The feature catalogue — one row per family, and the only place a feature name
//! becomes a family plus parameters (DATA-006 §7; AGENT-011 §2, P2, P6).
//!
//! # Why a registry rather than a list of names
//!
//! Before this module the platform had three disagreeing answers to "what
//! features exist": `feature_sets::known_features_static()` (a hand-written list
//! of 38 names), `runtime::construct` (a `match` over prefixes, which accepted
//! names the list did not contain), and `backtest::requirements::parse_feature`
//! (which accepted `ema_N` and `rsi_N` **only**, and refused everything else the
//! other two could compute). The third was the binding one: a strategy naming
//! `zscore_20` was refused before a bar was loaded, even though the engine had
//! computed `zscore_20` for the training frame all along. That gap — not the
//! engine — was the platform's capability ceiling.
//!
//! So: one table. [`resolve`] turns a name into a [`Resolved`] family and
//! parameters, [`families`] serves the catalogue with everything a model needs to
//! choose (`list_features`, AGENT-011 §4), and `runtime::construct` builds only
//! what `resolve` admits. A family that is not here does not exist anywhere, and
//! a family that is here is computable on every path.
//!
//! # The naming scheme
//!
//! `family` for a nullary feature (`close`, `obv`, `hour_sin`), or
//! `family_p1[_p2…]` with **integer** parameters in declaration order
//! (`ema_7`, `macd_12_26`, `bb_upper_20_2`). Integers only, deliberately: the
//! whole catalogue is then writable under a constrained grammar by a 7B
//! (AGENT-011 P7), and a name is its own canonical form, so `feature_set_id`
//! stays a hash over names.
//!
//! # What is *not* here, and why
//!
//! - **Source selection** (`rolling_mean(high, 20)`). Families whose meaning
//!   depends on reading a bar field other than the close have their own name —
//!   `atr_N`, `donchian_upper_N`, `parkinson_vol_N` — rather than a source
//!   argument. A source argument would double the grammar for a handful of
//!   combinations that are already named.
//! - **Exchange sessions.** DATA-006 §5 wants `session(name)` "instrument
//!   calendar aware". There is no instrument calendar in this crate and it may
//!   not acquire one without breaking purity, so what is registered is
//!   `utc_session_*` — fixed UTC hour windows, named for what they are. A
//!   feature called `session_nyse` that did not read the NYSE calendar would be
//!   worse than no feature at all.

use crate::PIT_LAG_BAR_DERIVED_MS;

/// What a feature costs to compute, so a model can choose (DATA-006 §7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CostClass {
    /// O(1) per bar, no window scan: passthrough and calendar features.
    Cheap,
    /// O(window) per bar: everything with a lookback.
    Rolling,
    /// Requires a model artifact or an external series. None in F1/F2.
    Model,
}

impl CostClass {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cheap => "cheap",
            Self::Rolling => "rolling",
            Self::Model => "model",
        }
    }
}

/// Which catalogue phase a family belongs to (AGENT-011 §2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    F1,
    F2,
}

/// One integer parameter of a family, with the range outside which the name does
/// not resolve at all.
#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub name: &'static str,
    pub min: usize,
    pub max: usize,
}

impl Param {
    const fn new(name: &'static str, min: usize, max: usize) -> Self {
        Self { name, min, max }
    }
}

/// A period parameter: the conventional 1…100 000 bar range.
const fn period(name: &'static str) -> Param {
    Param::new(name, 1, 100_000)
}

/// How a family's declared lookback follows from its parameters.
///
/// This is the honest-declaration point (INV-13): the value here is what
/// `register_check` (AT-13) tests the implementation against, and a family that
/// reads further back than it says fails registration rather than serving a
/// value computed from bars it was not entitled to.
#[derive(Clone, Copy, Debug)]
pub enum Lookback {
    /// Exactly `n` bars, independent of parameters.
    Fixed(u32),
    /// `params[i] * mult + plus`.
    Scaled { i: usize, mult: usize, plus: usize },
    /// `max(params[i] …) * mult + plus` — for families with several periods.
    MaxScaled { mult: usize, plus: usize },
    /// `params[i] + params[j] * mult + plus` — a window over a windowed input,
    /// where the inner feature's own lookback is part of the outer window.
    Nested { outer: usize, inner: usize, mult: usize, plus: usize },
    /// `(params[i] + params[j]) * mult + plus` — a windowed feature *of* a
    /// windowed feature, where both windows are scaled the same way. The macd
    /// signal line is the case: `signal` exponential steps over a macd series,
    /// each macd value needing `slow * mult` bars of its own.
    SumScaled { i: usize, j: usize, mult: usize, plus: usize },
}

impl Lookback {
    fn bars(self, params: &[usize]) -> Option<u32> {
        let v = match self {
            Self::Fixed(n) => return Some(n),
            Self::Scaled { i, mult, plus } => params.get(i)?.checked_mul(mult)?.checked_add(plus)?,
            Self::MaxScaled { mult, plus } => params.iter().max()?.checked_mul(mult)?.checked_add(plus)?,
            Self::Nested { outer, inner, mult, plus } => params
                .get(outer)?
                .checked_add(params.get(inner)?.checked_mul(mult)?)?
                .checked_add(plus)?,
            Self::SumScaled { i, j, mult, plus } => params.get(i)?.checked_add(*params.get(j)?)?.checked_mul(mult)?.checked_add(plus)?,
        };
        u32::try_from(v).ok()
    }
}

/// One catalogue row (DATA-006 §7): parameters and ranges, output dtype, warm-up
/// bars, PIT lag, cost class, and a one-line "use when".
#[derive(Clone, Copy, Debug)]
pub struct Family {
    /// The name prefix, and the whole name when `params` is empty.
    pub name: &'static str,
    pub params: &'static [Param],
    pub lookback: Lookback,
    pub output_dtype: &'static str,
    /// Publication lag of the *source*, in milliseconds. Zero for everything
    /// derived from a closed bar; a family over a lagged source (a filing, a
    /// revision-bearing macro series) declares the source's lag here and the
    /// value travels onto [`dataplane::feature::FeatureDef::knowledge_lag_ms`],
    /// so the PIT check has something to check.
    pub pit_lag_ms: u64,
    pub cost: CostClass,
    pub phase: Phase,
    /// One line, imperative, ≤ 120 chars: when a model should reach for it.
    pub use_when: &'static str,
    /// Another family computing the identical value under a different name. An
    /// alias is *not* a second implementation: `resolve` rewrites it to the
    /// canonical family before anything is constructed (INV-14).
    pub alias_of: Option<&'static str>,
}

impl Family {
    const fn new(
        name: &'static str,
        params: &'static [Param],
        lookback: Lookback,
        cost: CostClass,
        phase: Phase,
        use_when: &'static str,
    ) -> Self {
        Self {
            name,
            params,
            lookback,
            output_dtype: "f64",
            pit_lag_ms: PIT_LAG_BAR_DERIVED_MS,
            cost,
            phase,
            use_when,
            alias_of: None,
        }
    }

    const fn alias(mut self, of: &'static str) -> Self {
        self.alias_of = Some(of);
        self
    }

    /// Declared lookback for a parameter vector, in bars.
    #[must_use]
    pub fn lookback_bars(&self, params: &[usize]) -> Option<u32> {
        self.lookback.bars(params)
    }

    /// The name this family takes for `params`, e.g. `("ema", [7]) -> "ema_7"`.
    #[must_use]
    pub fn name_for(&self, params: &[usize]) -> String {
        if params.is_empty() {
            return self.name.to_string();
        }
        let mut s = self.name.to_string();
        for p in params {
            s.push('_');
            s.push_str(&p.to_string());
        }
        s
    }

    /// A representative name, using each parameter's documented example value.
    #[must_use]
    pub fn example(&self) -> String {
        self.name_for(&self.example_params())
    }

    /// Example parameter values — the midpoint of convention, not of the range.
    #[must_use]
    pub fn example_params(&self) -> Vec<usize> {
        self.params.iter().map(|p| EXAMPLES.iter().find(|(n, _)| *n == p.name).map_or(p.min.max(14), |(_, v)| (*v).clamp(p.min, p.max))).collect()
    }
}

/// Conventional example values per parameter name, so `list_features` shows a
/// name a model can paste rather than a range it has to instantiate.
const EXAMPLES: &[(&str, usize)] = &[
    ("n", 20),
    ("fast", 12),
    ("slow", 26),
    ("signal", 9),
    ("k", 2),
    ("q", 50),
    ("halflife", 10),
    ("lag", 1),
    ("horizon", 4),
];

/// A resolved name: the family it belongs to and its parameters.
#[derive(Clone, Copy, Debug)]
pub struct Resolved {
    /// The **canonical** family — aliases are already rewritten.
    pub family: &'static Family,
    params: [usize; MAX_PARAMS],
    n: usize,
}

/// No family in the catalogue takes more than this many parameters. A flat array
/// keeps `Resolved` `Copy`, which keeps it out of the allocator on the hot path.
pub const MAX_PARAMS: usize = 3;

impl Resolved {
    #[must_use]
    pub fn params(&self) -> &[usize] {
        &self.params[..self.n]
    }

    /// Declared lookback, in bars.
    #[must_use]
    pub fn lookback_bars(&self) -> Option<u32> {
        self.family.lookback_bars(self.params())
    }
}

macro_rules! p {
    ($($x:expr),* $(,)?) => { &[$($x),*] };
}

/// Every family the platform can compute. **The** list — `runtime::construct`
/// builds nothing this does not admit, and `list_features` serves nothing else.
#[rustfmt::skip]
static FAMILIES: &[Family] = &[
    // ── Passthrough (DATA-006 §4 `field`) ────────────────────────────────────
    Family::new("open",   p![], Lookback::Fixed(1), CostClass::Cheap, Phase::F1, "the bar's open; use as the reference for an intrabar move"),
    Family::new("high",   p![], Lookback::Fixed(1), CostClass::Cheap, Phase::F1, "the bar's high"),
    Family::new("low",    p![], Lookback::Fixed(1), CostClass::Cheap, Phase::F1, "the bar's low"),
    Family::new("close",  p![], Lookback::Fixed(1), CostClass::Cheap, Phase::F1, "the bar's close; the default source of every windowed family"),
    Family::new("volume", p![], Lookback::Fixed(1), CostClass::Cheap, Phase::F1, "the bar's volume; compare with rel_volume_N rather than an absolute level"),

    // ── Returns, lags, differences (DATA-006 §5 Returns) ─────────────────────
    Family::new("returns",     p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 1 }, CostClass::Rolling, Phase::F1, "simple return over n bars; the usual momentum primitive"),
    Family::new("momentum",    p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 1 }, CostClass::Rolling, Phase::F1, "same value as returns_n").alias("returns"),
    Family::new("ret",         p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 1 }, CostClass::Rolling, Phase::F1, "same value as returns_n (DATA-006 spelling)").alias("returns"),
    Family::new("log_returns", p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 1 }, CostClass::Rolling, Phase::F1, "log return over n bars; additive across bars, so use it when summing returns"),
    Family::new("logret",      p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 1 }, CostClass::Rolling, Phase::F1, "same value as log_returns_n").alias("log_returns"),
    Family::new("lag",         p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 1 }, CostClass::Rolling, Phase::F1, "the close n bars ago; compare a level with its own past"),
    Family::new("diff",        p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 1 }, CostClass::Rolling, Phase::F1, "close minus close n bars ago, in price units, not a ratio"),

    // ── Rolling statistics (DATA-006 §5 Rolling) ─────────────────────────────
    Family::new("rolling_mean",     p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "mean close over n bars; the trend baseline a price is compared against"),
    Family::new("sma",              p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "same value as rolling_mean_n").alias("rolling_mean"),
    Family::new("rolling_std",      p![Param::new("n", 2, 100_000)], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "sample standard deviation of close over n bars; the sigma unit of an exit distance"),
    Family::new("rolling_var",      p![Param::new("n", 2, 100_000)], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "sample variance of close over n bars"),
    Family::new("rolling_min",      p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "lowest close over n bars"),
    Family::new("rolling_max",      p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "highest close over n bars"),
    Family::new("rolling_sum",      p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "sum of close over n bars"),
    Family::new("rolling_median",   p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "median close over n bars; an outlier-robust baseline"),
    Family::new("rolling_quantile", p![period("n"), Param::new("q", 0, 100)], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "the q-th percentile close over n bars, q in whole percent"),
    Family::new("rolling_skew",     p![Param::new("n", 3, 100_000)], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "sample skewness of close over n bars"),
    Family::new("rolling_kurt",     p![Param::new("n", 4, 100_000)], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "excess kurtosis of close over n bars; high values mean fat tails"),
    Family::new("zscore",           p![Param::new("n", 2, 100_000)], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "how many sigma the close is from its n-bar mean; the mean-reversion primitive"),
    Family::new("pct_rank",         p![Param::new("n", 2, 100_000)], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "fraction of the last n closes below this one, 0..1; use to gate on a regime percentile"),
    Family::new("ewma",             p![period("halflife")], Lookback::Scaled { i: 0, mult: EWMA_LOOKBACK_HALFLIVES, plus: 0 }, CostClass::Rolling, Phase::F1, "exponentially weighted mean with the given halflife in bars"),

    // ── Indicators (DATA-006 §5 Nautilus wrappers) ───────────────────────────
    Family::new("ema",            p![period("n")], Lookback::Scaled { i: 0, mult: crate::ema::EMA_LOOKBACK_PERIODS, plus: 0 }, CostClass::Rolling, Phase::F1, "exponential moving average of close over n bars"),
    Family::new("rsi",            p![Param::new("n", 2, 100_000)], Lookback::Scaled { i: 0, mult: crate::rsi::RSI_LOOKBACK_PERIODS, plus: 1 }, CostClass::Rolling, Phase::F1, "Wilder RSI of close, 0..100; below 30 is the conventional oversold gate"),
    Family::new("atr",            p![Param::new("n", 2, 100_000)], Lookback::Scaled { i: 0, mult: crate::rsi::RSI_LOOKBACK_PERIODS, plus: 1 }, CostClass::Rolling, Phase::F1, "Wilder average true range in price units; the unit an exit distance should use"),
    Family::new("roc",            p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 1 }, CostClass::Rolling, Phase::F1, "rate of change over n bars, in percent"),
    Family::new("macd",           p![period("fast"), period("slow")], Lookback::MaxScaled { mult: crate::ema::EMA_LOOKBACK_PERIODS, plus: 0 }, CostClass::Rolling, Phase::F1, "ema_fast minus ema_slow; positive means the fast trend leads"),
    Family::new("macd_signal",    p![period("fast"), period("slow"), period("signal")], Lookback::SumScaled { i: 1, j: 2, mult: crate::ema::EMA_LOOKBACK_PERIODS, plus: 0 }, CostClass::Rolling, Phase::F1, "the signal-period ema of macd_fast_slow"),
    Family::new("macd_hist",      p![period("fast"), period("slow"), period("signal")], Lookback::SumScaled { i: 1, j: 2, mult: crate::ema::EMA_LOOKBACK_PERIODS, plus: 0 }, CostClass::Rolling, Phase::F1, "macd minus its signal line; the crossover primitive"),
    Family::new("bb_upper",       p![Param::new("n", 2, 100_000), Param::new("k", 1, 10)], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "rolling mean plus k sample standard deviations of close"),
    Family::new("bb_lower",       p![Param::new("n", 2, 100_000), Param::new("k", 1, 10)], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "rolling mean minus k sample standard deviations of close"),
    Family::new("bb_width",       p![Param::new("n", 2, 100_000), Param::new("k", 1, 10)], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "band width as a fraction of the mean; a squeeze is a low value"),
    Family::new("bb_pctb",        p![Param::new("n", 2, 100_000), Param::new("k", 1, 10)], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "where close sits in the band, 0 at the lower and 1 at the upper"),
    Family::new("donchian_upper", p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "highest high over n bars; the breakout level"),
    Family::new("donchian_lower", p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "lowest low over n bars"),
    Family::new("donchian_mid",   p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "midpoint of the n-bar high-low channel"),
    Family::new("donchian_width", p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "channel height as a fraction of the midpoint"),
    Family::new("keltner_upper",  p![Param::new("n", 2, 100_000), Param::new("k", 1, 10)], Lookback::Scaled { i: 0, mult: crate::ema::EMA_LOOKBACK_PERIODS, plus: 1 }, CostClass::Rolling, Phase::F1, "ema_n plus k atr_n; a volatility band that does not assume normality"),
    Family::new("keltner_lower",  p![Param::new("n", 2, 100_000), Param::new("k", 1, 10)], Lookback::Scaled { i: 0, mult: crate::ema::EMA_LOOKBACK_PERIODS, plus: 1 }, CostClass::Rolling, Phase::F1, "ema_n minus k atr_n"),
    Family::new("obv",            p![], Lookback::Fixed(crate::runtime::OBV_WINDOW), CostClass::Rolling, Phase::F1, "on-balance volume over a fixed 256-bar window"),
    Family::new("rel_volume",     p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F1, "this bar's volume over the n-bar mean; 1.0 is an ordinary bar"),

    // ── Calendar (DATA-006 §5 Calendar) ──────────────────────────────────────
    Family::new("hour_sin",       p![], Lookback::Fixed(1), CostClass::Cheap, Phase::F1, "sine of the UTC hour; pair with hour_cos so midnight is next to 23:00"),
    Family::new("hour_cos",       p![], Lookback::Fixed(1), CostClass::Cheap, Phase::F1, "cosine of the UTC hour"),
    Family::new("dow_sin",        p![], Lookback::Fixed(1), CostClass::Cheap, Phase::F1, "sine of the UTC day of week"),
    Family::new("dow_cos",        p![], Lookback::Fixed(1), CostClass::Cheap, Phase::F1, "cosine of the UTC day of week"),
    Family::new("utc_session_asia",   p![], Lookback::Fixed(1), CostClass::Cheap, Phase::F1, "1 inside 00:00-08:00 UTC, else 0; a UTC window, not an exchange calendar"),
    Family::new("utc_session_london", p![], Lookback::Fixed(1), CostClass::Cheap, Phase::F1, "1 inside 07:00-16:00 UTC, else 0; a UTC window, not an exchange calendar"),
    Family::new("utc_session_us",     p![], Lookback::Fixed(1), CostClass::Cheap, Phase::F1, "1 inside 13:00-22:00 UTC, else 0; a UTC window, not an exchange calendar"),

    // ── Range volatility (DATA-006 §5 Range volatility) — F2 ─────────────────
    Family::new("parkinson_vol",       p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F2, "Parkinson range volatility over n bars; not annualised"),
    Family::new("garman_klass_vol",    p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F2, "Garman-Klass volatility over n bars; uses the open as well as the range"),
    Family::new("rogers_satchell_vol", p![period("n")], Lookback::Scaled { i: 0, mult: 1, plus: 0 }, CostClass::Rolling, Phase::F2, "Rogers-Satchell volatility over n bars; drift-independent"),
    Family::new("rv",                  p![Param::new("n", 2, 100_000)], Lookback::Scaled { i: 0, mult: 1, plus: 1 }, CostClass::Rolling, Phase::F2, "realised volatility: root sum of squared bar log returns over n bars"),
    Family::new("bv",                  p![Param::new("n", 3, 100_000)], Lookback::Scaled { i: 0, mult: 1, plus: 1 }, CostClass::Rolling, Phase::F2, "bipower variation over n bars; jump-robust, so rv minus bv isolates jumps"),
    Family::new("jump_share",          p![Param::new("n", 3, 100_000)], Lookback::Scaled { i: 0, mult: 1, plus: 1 }, CostClass::Rolling, Phase::F2, "share of realised variance attributable to jumps, 0..1"),
    Family::new("autocorr",            p![Param::new("n", 4, 100_000), Param::new("lag", 1, 1_000)], Lookback::Nested { outer: 0, inner: 1, mult: 1, plus: 1 }, CostClass::Rolling, Phase::F2, "autocorrelation of bar returns at the given lag over n bars"),
    Family::new("variance_ratio",      p![Param::new("n", 8, 100_000), Param::new("horizon", 2, 1_000)], Lookback::Nested { outer: 0, inner: 1, mult: 2, plus: 1 }, CostClass::Rolling, Phase::F2, "Lo-MacKinlay variance ratio at horizon q; above 1 is trending, below 1 mean-reverting"),
    Family::new("efficiency_ratio",    p![Param::new("n", 2, 100_000)], Lookback::Scaled { i: 0, mult: 1, plus: 1 }, CostClass::Rolling, Phase::F2, "net move over total path length, 0..1; the persistence filter a trend rule wants"),
];

/// Bars of window per halflife for [`ewma`](FAMILIES), on the same argument as
/// `EMA_LOOKBACK_PERIODS`: a weight that never expires cannot be declared.
/// After `n` halflives the seed's residual weight is 2^-n; at 8 that is 0.4%.
pub const EWMA_LOOKBACK_HALFLIVES: usize = 8;

/// The catalogue, in declaration order.
#[must_use]
pub fn families() -> &'static [Family] {
    FAMILIES
}

/// The family with this exact name, alias or not.
#[must_use]
pub fn family(name: &str) -> Option<&'static Family> {
    FAMILIES.iter().find(|f| f.name == name)
}

/// Turn a feature name into its canonical family and parameters, or `None` when
/// no family answers to it or a parameter is outside its declared range.
///
/// Parsing is longest-prefix-first, so `macd_signal_12_26_9` resolves to
/// `macd_signal` rather than to `macd` with a junk parameter.
#[must_use]
pub fn resolve(name: &str) -> Option<Resolved> {
    let mut best: Option<(&'static Family, Vec<usize>)> = None;
    for f in FAMILIES {
        let Some(rest) = name.strip_prefix(f.name) else { continue };
        let params: Vec<usize> = if rest.is_empty() {
            vec![]
        } else {
            let Some(tail) = rest.strip_prefix('_') else { continue };
            let Ok(parsed) = tail.split('_').map(str::parse::<usize>).collect::<Result<Vec<_>, _>>() else { continue };
            parsed
        };
        if params.len() != f.params.len() {
            continue;
        }
        if params.iter().zip(f.params).any(|(v, spec)| *v < spec.min || *v > spec.max) {
            continue;
        }
        if best.as_ref().is_none_or(|(b, _)| f.name.len() > b.name.len()) {
            best = Some((f, params));
        }
    }
    let (f, params) = best?;
    let canonical = f.alias_of.and_then(family).unwrap_or(f);
    if params.len() > MAX_PARAMS {
        return None;
    }
    let mut arr = [0usize; MAX_PARAMS];
    arr[..params.len()].copy_from_slice(&params);
    // A resolved name whose lookback cannot be expressed is not resolved: the
    // executor would have nothing to enforce coverage against (INV-13).
    let r = Resolved { family: canonical, params: arr, n: params.len() };
    r.lookback_bars()?;
    Some(r)
}

/// Whether `name` is a catalogued feature.
#[must_use]
pub fn is_known(name: &str) -> bool {
    resolve(name).is_some()
}

/// A filtered view of the catalogue, for `list_features` (AGENT-011 §4).
///
/// `query` matches the family name or its `use_when` text, case-insensitively.
#[must_use]
pub fn list(family_filter: Option<&str>, cost: Option<CostClass>, query: Option<&str>) -> Vec<&'static Family> {
    let q = query.map(str::to_lowercase);
    FAMILIES
        .iter()
        .filter(|f| family_filter.is_none_or(|want| f.name == want))
        .filter(|f| cost.is_none_or(|want| f.cost == want))
        .filter(|f| {
            q.as_ref().is_none_or(|q| {
                f.name.to_lowercase().contains(q)
                    || f.use_when.to_lowercase().contains(q)
                    || f.alias_of.is_some_and(|a| a.to_lowercase().contains(q))
            })
        })
        .collect()
}

/// The catalogued families a misspelling is closest to, best first, at most three.
///
/// Feeds `FeatureError::Unknown`'s suggestions (DATA-006 §8) and the validator's
/// `unknown_feature` fix (AGENT-011 §5). Edit distance over the family prefix
/// only: a model that writes `rsi14` has the family right and the syntax wrong,
/// which is the common case and the one worth answering well.
#[must_use]
pub fn suggestions(name: &str) -> Vec<&'static Family> {
    let stem: String = name.chars().take_while(|c| !c.is_ascii_digit()).collect();
    let stem = stem.trim_end_matches('_').to_lowercase();
    let mut scored: Vec<(usize, &'static Family)> = FAMILIES.iter().map(|f| (edit_distance(&stem, f.name), f)).filter(|(d, f)| *d <= 3 || f.name.starts_with(&stem)).collect();
    scored.sort_by_key(|(d, f)| (*d, f.name));
    scored.into_iter().take(3).map(|(_, f)| f).collect()
}

/// Levenshtein distance, two rows.
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_family_name_is_unique_and_its_use_when_fits_one_line() {
        let mut seen = std::collections::BTreeSet::new();
        for f in families() {
            assert!(seen.insert(f.name), "duplicate family `{}`", f.name);
            assert!(f.use_when.len() <= 120, "`{}` use_when is {} chars", f.name, f.use_when.len());
            assert!(!f.use_when.is_empty(), "`{}` has no use_when", f.name);
            assert!(f.params.len() <= MAX_PARAMS, "`{}` takes {} params", f.name, f.params.len());
        }
    }

    /// An alias must name a family that exists and take the same parameters, or
    /// `resolve` would rewrite a name into one that cannot be built.
    #[test]
    fn every_alias_points_at_a_real_family_with_the_same_arity() {
        for f in families() {
            let Some(target) = f.alias_of else { continue };
            let t = family(target).unwrap_or_else(|| panic!("`{}` aliases missing `{target}`", f.name));
            assert!(t.alias_of.is_none(), "`{}` aliases `{target}`, which is itself an alias", f.name);
            assert_eq!(f.params.len(), t.params.len(), "`{}` and `{target}` differ in arity", f.name);
        }
    }

    #[test]
    fn resolve_reads_the_parameters_in_declaration_order() {
        let r = resolve("macd_signal_12_26_9").unwrap();
        assert_eq!(r.family.name, "macd_signal");
        assert_eq!(r.params(), [12, 26, 9]);
    }

    /// Longest prefix wins, or `macd_signal_12_26_9` would resolve to `macd`.
    #[test]
    fn a_longer_family_name_wins_over_a_prefix_of_it() {
        assert_eq!(resolve("macd_12_26").unwrap().family.name, "macd");
        assert_eq!(resolve("macd_hist_12_26_9").unwrap().family.name, "macd_hist");
        assert_eq!(resolve("rolling_mean_20").unwrap().family.name, "rolling_mean");
    }

    #[test]
    fn an_alias_resolves_to_the_canonical_family() {
        assert_eq!(resolve("sma_20").unwrap().family.name, "rolling_mean");
        assert_eq!(resolve("momentum_5").unwrap().family.name, "returns");
        assert_eq!(resolve("ret_5").unwrap().family.name, "returns");
        assert_eq!(resolve("logret_1").unwrap().family.name, "log_returns");
    }

    /// The range is part of the name's meaning: a parameter outside it does not
    /// resolve at all, rather than resolving to something the engine clamps.
    #[test]
    fn a_parameter_outside_its_declared_range_does_not_resolve() {
        assert!(resolve("rsi_1").is_none(), "rsi needs at least two bars of change");
        assert!(resolve("rolling_skew_2").is_none());
        assert!(resolve("rolling_quantile_20_101").is_none());
        assert!(resolve("bb_upper_20_0").is_none());
        assert!(resolve("ema_0").is_none());
        assert!(resolve("ema_100001").is_none());
    }

    #[test]
    fn a_wrong_arity_does_not_resolve() {
        assert!(resolve("ema").is_none());
        assert!(resolve("ema_7_9").is_none());
        assert!(resolve("close_20").is_none());
        assert!(resolve("macd_12").is_none());
    }

    #[test]
    fn a_nonsense_name_does_not_resolve() {
        for bad in ["", "ema_x", "not_a_feature", "rsi14", "_ema_7", "ema_-7"] {
            assert!(resolve(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn lookback_follows_the_parameters() {
        assert_eq!(resolve("ema_20").unwrap().lookback_bars(), Some(100));
        assert_eq!(resolve("rsi_14").unwrap().lookback_bars(), Some(71));
        assert_eq!(resolve("rolling_mean_20").unwrap().lookback_bars(), Some(20));
        assert_eq!(resolve("returns_5").unwrap().lookback_bars(), Some(6));
        // macd spans the slower of its two emas.
        assert_eq!(resolve("macd_12_26").unwrap().lookback_bars(), Some(130));
        // the signal ema runs over the macd series, so both windows compose.
        assert_eq!(resolve("macd_signal_12_26_9").unwrap().lookback_bars(), Some(175));
        assert_eq!(resolve("close").unwrap().lookback_bars(), Some(1));
    }

    #[test]
    fn list_filters_on_family_cost_and_query() {
        assert_eq!(list(Some("ema"), None, None).len(), 1);
        assert!(list(None, Some(CostClass::Cheap), None).iter().all(|f| f.cost == CostClass::Cheap));
        let rsi = list(None, None, Some("rsi"));
        assert!(rsi.iter().any(|f| f.name == "rsi"));
        assert!(list(None, None, Some("oversold")).iter().any(|f| f.name == "rsi"));
        assert!(list(Some("no_such_family"), None, None).is_empty());
    }

    #[test]
    fn suggestions_answer_the_common_misspellings() {
        assert_eq!(suggestions("rsi14")[0].name, "rsi");
        assert_eq!(suggestions("ema")[0].name, "ema");
        assert!(suggestions("zscor").iter().any(|f| f.name == "zscore"));
        assert!(suggestions("qqqqqqqqqqqqqqq").is_empty());
    }

    #[test]
    fn an_example_name_resolves() {
        for f in families() {
            let name = f.example();
            assert!(resolve(&name).is_some(), "`{}` example `{name}` does not resolve", f.name);
        }
    }

    /// Everything derived from a closed bar has no publication lag; a family that
    /// declares one has a source, and the source's lag is what it must declare.
    #[test]
    fn every_f1_and_f2_family_is_bar_derived_and_declares_no_source_lag() {
        for f in families() {
            assert_eq!(f.pit_lag_ms, 0, "`{}` declares a source lag but reads bars", f.name);
            assert_eq!(f.output_dtype, "f64");
            assert_ne!(f.cost, CostClass::Model, "no F1/F2 family reads a model artifact");
        }
    }
}
