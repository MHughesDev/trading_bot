//! Run the check battery over every series the platform holds (P-2.6).
//!
//! **This is the task that must land before P-2.3's refusal is switched on.**
//! Enabling enforcement without first knowing how many series would be refused
//! is how a correctness improvement becomes an outage, so the sequence is:
//! publish the grades, count the blast radius, *then* decide.
//!
//! ```text
//! CLICKHOUSE_URL=http://trading:trading@localhost:8123 \
//!   cargo run -p dataplane --example qc_census
//! ```
//!
//! Reads `trading.market_bars_v2` directly rather than through `BarStore`,
//! because the census (P-0.1) found `market_bar` — the table `BarStore` reads —
//! does not exist in the live database, so every canonical read returns nothing.
//! That is recorded as a finding rather than worked around silently.

use std::collections::BTreeMap;

use dataplane::qc::{self, Grade, QcBar, QcConfig, QcContext, QcSubject};

const TSV: &str = "FORMAT TSVWithNames";

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL")
        .unwrap_or_else(|_| "http://trading:trading@localhost:8123".into())
}

/// One GET against ClickHouse. A POST body is interpreted as the query text and
/// a `query=` prefix makes it a syntax error, which cost a few minutes once.
fn query(sql: &str) -> Result<String, String> {
    let url = format!(
        "{}/?query={}",
        ch_url().trim_end_matches('/'),
        urlencode(sql)
    );
    let out = std::process::Command::new("curl")
        .args(["-s", "--max-time", "120", &url])
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    let body = String::from_utf8_lossy(&out.stdout).into_owned();
    if body.starts_with("Code:") {
        return Err(body.lines().next().unwrap_or(&body).to_string());
    }
    Ok(body)
}

fn urlencode(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

struct Series {
    instrument: String,
    venue: String,
    timeframe: String,
    bars: u64,
}

fn period_secs(tf: &str) -> i64 {
    match tf {
        "5m" => 300,
        "15m" => 900,
        "1h" => 3_600,
        "4h" => 14_400,
        "1d" => 86_400,
        // 1m and anything unrecognised: a minute is the platform's base grid,
        // and guessing wider would understate gaps.
        _ => 60,
    }
}

fn main() -> Result<(), String> {
    let inventory = query(&format!(
        "SELECT instrument_id, venue_id, timeframe, count() AS bars \
         FROM trading.market_bars_v2 GROUP BY instrument_id, venue_id, timeframe \
         ORDER BY bars DESC {TSV}"
    ))?;

    let series: Vec<Series> = inventory
        .lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() >= 4).then(|| Series {
                instrument: f[0].into(),
                venue: f[1].into(),
                timeframe: f[2].into(),
                bars: f[3].parse().unwrap_or(0),
            })
        })
        .collect();

    let cfg = QcConfig::default();
    println!("config version {}\n", cfg.version());
    println!(
        "{:<28} {:<10} {:<5} {:>8}  {:<5} first reason",
        "instrument", "venue", "tf", "bars", "grade"
    );
    let rule: String = std::iter::repeat_n('-', 118).collect();
    println!("{rule}");

    // Series keyed for the cross-source lookup: the same instrument and
    // timeframe on a different venue is the second opinion.
    let mut by_key: BTreeMap<(String, String), Vec<&Series>> = BTreeMap::new();
    for s in &series {
        by_key
            .entry((s.instrument.clone(), s.timeframe.clone()))
            .or_default()
            .push(s);
    }

    let mut tally: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut d_reasons: Vec<String> = Vec::new();

    for s in &series {
        let bars = load(&s.instrument, &s.venue, &s.timeframe)?;

        // The second venue, if one covers the same instrument and timeframe.
        let secondary = by_key
            .get(&(s.instrument.clone(), s.timeframe.clone()))
            .and_then(|v| v.iter().find(|o| o.venue != s.venue))
            .map(|o| load(&o.instrument, &o.venue, &o.timeframe))
            .transpose()?;

        // Expected bars come from the census in production. Here the window is
        // taken from the series itself, which measures *internal* gaps and not
        // absence at the ends — stated rather than glossed, because it makes
        // these coverage numbers optimistic.
        let period = period_secs(&s.timeframe);
        let expected = bars
            .first()
            .zip(bars.last())
            .map_or(s.bars, |(f, l)| {
                (((l.event_ns - f.event_ns) / 1_000_000_000 / period) + 1).max(1) as u64
            });

        let subject = QcSubject {
            instrument_id: s.instrument.clone(),
            venue_id: s.venue.clone(),
            timeframe: s.timeframe.clone(),
            from_ns: bars.first().map_or(0, |b| b.event_ns),
            to_ns: bars.last().map_or(0, |b| b.event_ns),
            expected_bars: expected,
            period_ns: period * 1_000_000_000,
        };
        let ctx = QcContext {
            secondary: secondary.as_deref(),
            ..QcContext::default()
        };

        let report = qc::run(&bars, &subject, &cfg, &ctx);
        let grade = report.graded.grade;
        *tally.entry(grade.as_str()).or_default() += 1;

        let first = report.graded.reasons.first().cloned().unwrap_or_default();
        println!(
            "{:<28} {:<10} {:<5} {:>8}  {:<5} {}",
            s.instrument,
            s.venue,
            s.timeframe,
            s.bars,
            grade.as_str(),
            first.chars().take(58).collect::<String>()
        );

        if grade == Grade::D {
            d_reasons.push(format!(
                "{}/{}/{}: {}",
                s.instrument, s.venue, s.timeframe, first
            ));
        }
    }

    println!("\n── blast radius ──");
    for g in ["A", "B", "C", "D"] {
        println!("  {g}: {}", tally.get(g).copied().unwrap_or(0));
    }
    let refused = tally.get("D").copied().unwrap_or(0);
    let waiver = tally.get("C").copied().unwrap_or(0);
    println!(
        "\n{refused} series would be REFUSED outright and {waiver} would need a human waiver \
         if P-2.3 were switched on today, out of {}.",
        series.len()
    );
    if !d_reasons.is_empty() {
        println!("\nWhy the D grades:");
        for r in &d_reasons {
            println!("  - {r}");
        }
    }
    Ok(())
}

fn load(instrument: &str, venue: &str, timeframe: &str) -> Result<Vec<QcBar>, String> {
    let sql = format!(
        "SELECT toUnixTimestamp64Nano(event_time), toUnixTimestamp64Nano(available_time), \
                toFloat64(open), toFloat64(high), toFloat64(low), toFloat64(close), \
                toFloat64(volume), revision \
         FROM trading.market_bars_v2 \
         WHERE instrument_id = '{instrument}' AND venue_id = '{venue}' AND timeframe = '{timeframe}' \
         ORDER BY event_time FORMAT TSV"
    );
    Ok(query(&sql)?
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() >= 8).then(|| QcBar {
                event_ns: f[0].parse().unwrap_or(0),
                knowledge_ns: f[1].parse().unwrap_or(0),
                open: f[2].parse().unwrap_or(0.0),
                high: f[3].parse().unwrap_or(0.0),
                low: f[4].parse().unwrap_or(0.0),
                close: f[5].parse().unwrap_or(0.0),
                volume: f[6].parse().unwrap_or(0.0),
                revised: f[7].parse::<u32>().unwrap_or(0) > 0,
            })
        })
        .collect())
}
