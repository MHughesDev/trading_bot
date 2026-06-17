//! Compute a model's feature vector from recent ClickHouse bars.
//!
//! Mirrors the trainer's `features.py` so the Test Lab feeds a model the same
//! columns it was trained on — and the same ones the live inference path
//! produces (this uses the shared `features` crate primitives).

use backtest::store::LoadedBar;
use features::{Ema, Rsi};
use rust_decimal::prelude::ToPrimitive;
use serde_json::{json, Map, Value};

fn dec(d: rust_decimal::Decimal) -> f64 {
    d.to_f64().unwrap_or(0.0)
}

/// Compute the latest-bar value for each requested feature name.
///
/// Unknown feature names resolve to 0.0 (the inference side fills missing
/// features with 0 anyway, so this stays consistent).
pub fn latest_vector(bars: &[LoadedBar], names: &[String]) -> Map<String, Value> {
    let closes: Vec<f64> = bars.iter().map(|b| dec(b.close)).collect();
    let mut out = Map::new();
    for name in names {
        out.insert(name.clone(), json!(compute_one(name, bars, &closes)));
    }
    out
}

fn compute_one(name: &str, bars: &[LoadedBar], closes: &[f64]) -> f64 {
    let n = closes.len();
    if n == 0 {
        return 0.0;
    }
    let last = n - 1;
    let last_bar = &bars[last];

    match name {
        "open" => dec(last_bar.open),
        "high" => dec(last_bar.high),
        "low" => dec(last_bar.low),
        "close" => dec(last_bar.close),
        "volume" => dec(last_bar.volume),
        "log_returns_1" => {
            if n >= 2 && closes[last - 1] > 0.0 {
                (closes[last] / closes[last - 1]).ln()
            } else {
                0.0
            }
        }
        _ if name.starts_with("ema_") => match parse_period(name) {
            Some(p) => {
                let mut e = Ema::new(p.max(1));
                let mut v = closes[0];
                for &c in closes {
                    v = e.update(c);
                }
                v
            }
            None => 0.0,
        },
        _ if name.starts_with("rsi_") => match parse_period(name) {
            Some(p) if p >= 2 => {
                let mut r = Rsi::new(p);
                let mut latest = 0.0;
                for &c in closes {
                    if let Some(v) = r.update(c) {
                        latest = v;
                    }
                }
                latest
            }
            _ => 0.0,
        },
        _ if name.starts_with("rolling_mean_") => match parse_period(name) {
            Some(p) => window_mean(closes, p),
            None => 0.0,
        },
        _ if name.starts_with("rolling_std_") => match parse_period(name) {
            Some(p) => window_std(closes, p),
            None => 0.0,
        },
        _ if name.starts_with("returns_") => match parse_period(name) {
            Some(p) if n > p && closes[last - p] != 0.0 => closes[last] / closes[last - p] - 1.0,
            _ => 0.0,
        },
        _ => 0.0,
    }
}

/// Parse the trailing integer of a feature name like `ema_7` or `rolling_mean_14`.
fn parse_period(name: &str) -> Option<usize> {
    name.rsplit('_').next().and_then(|s| s.parse().ok())
}

fn window_mean(closes: &[f64], period: usize) -> f64 {
    if period == 0 || closes.len() < period {
        return 0.0;
    }
    let w = &closes[closes.len() - period..];
    w.iter().sum::<f64>() / period as f64
}

/// Sample standard deviation (ddof=1), matching pandas `.rolling(period).std()`.
fn window_std(closes: &[f64], period: usize) -> f64 {
    if period < 2 || closes.len() < period {
        return 0.0;
    }
    let w = &closes[closes.len() - period..];
    let mean = w.iter().sum::<f64>() / period as f64;
    let var = w.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / (period as f64 - 1.0);
    var.sqrt()
}
