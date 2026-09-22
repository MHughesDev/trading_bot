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
//! Reads the canonical `market_bar`. It used to read `trading.market_bars_v2`, on
//! the strength of a P-0.1 finding that `market_bar` "does not exist in the live
//! database". That premise expired: re-measured on 2026-09-22, `market_bar` held
//! 170,924 rows with its newest event at 02:05 that morning, while
//! `market_bars_v2`'s newest was 2026-09-14 01:00 — eight days stale. The census
//! was grading a table nothing had written in a week.
//!
//! `market_bars_v2` is now frozen (ADR-D-29) and may not answer a point-in-time
//! question: `available_time` is absent from its sorting key, so a restatement of a
//! bar collapses into its original on the next merge and leaves nothing to read.
//! Measured on the live table the same day, 1,441 sorting-key groups already held
//! two physical rows each — 1,441 rows queued for silent destruction. `market_bar`
//! carries `knowledge_time` *inside* its sorting key and keeps both rows: it holds
//! zero groups that share a full sorting key, so it has no equivalent hazard.
//! `cargo xtask check-bars-v2-frozen` keeps new writers off v2.
//!
//! Three consequences for the numbers this prints, all real rather than cosmetic:
//!
//! - **Every grade published from the v2 table must be re-measured.** The P-2.6
//!   census grades were taken on the stale table and do not describe the platform's
//!   data as it stands. Nothing below re-publishes them; they are simply void.
//! - `market_bar` stores the bar's **open** time where v2 stored its **close**, and
//!   `QcBar::event_ns` means the open, so `load` maps the column straight across
//!   with no shift. That is the correct mapping and the old one was the wrong one.
//! - It does, however, *weaken* the one knowledge-lag rule the battery enforces.
//!   `qc::checks::timestamps` only refuses `knowledge_ns < event_ns`; fed closes
//!   it was incidentally refusing bars knowable before their close, and fed opens it
//!   no longer is. That slack is presently unused rather than merely unexamined:
//!   across all 170,924 rows of `market_bar`, zero have
//!   `knowledge_time < event_time + period_secs`, and the tightest series
//!   (BTC-USD/kraken/1m over `kraken_ws`) sits exactly on the boundary at a 60s lag.
//!   A measured zero, not an absent measurement — but a bar knowable mid-period
//!   would now pass, and closing that needs the period inside the check.
//! - **A generated series is graded but not counted.** `market_bar` serves
//!   `SYN-PLANTED-AR1-424242` — 5,000 1h bars from the `synthetic` source, written by
//!   `crates/api/src/synthetic_worker.rs` through the same `BarStore` as vendor data —
//!   beside real venue feeds, and it is the longest-spanning series in the table. It
//!   is printed under `kind = SYN` and kept out of the blast-radius total, which is a
//!   claim about market data. Detection is on `source_dim.kind = 'derived'`, not on
//!   the name: a fixture renamed tomorrow is still a fixture.
//!
//! # What the canonical table actually grades (2026-09-22)
//!
//! Of 8 live venue series: **0 A, 2 B, 2 C, 4 D**. Every D is coverage — 43% to 99.99%
//! of expected bars missing — not a timestamp or price fault, and both Bs are tiny
//! series (121 and 48 bars) where `cross_source` and `survivorship` could not run at
//! all, so the grade reflects what went unmeasured rather than what passed.
//!
//! Stated plainly, because the number reads better than the situation: the
//! best-attested series in this census is the synthetic fixture, and it is excluded.
//! No live venue series earns an A. These grades replace the P-2.6 figures, which were
//! taken on `market_bars_v2` and are void; they are not comparable with them, and the
//! difference is mostly the canonical table honestly showing gaps the stale one had no
//! rows to reveal.

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
    /// Surrogate keys, as `market_bar` actually stores them. Selection is done on
    /// these, so a series whose name will not resolve is still censused.
    instrument_id: i64,
    venue_id: i32,
    period_secs: i64,
    /// Display, and the key for the cross-venue second opinion.
    instrument: String,
    venue: String,
    timeframe: String,
    bars: u64,
    /// Every bar of this series came from a source whose `source_dim.kind` is
    /// `derived` — a generator, not a venue.
    ///
    /// Taken from the source dimension rather than matched against a name, because
    /// the thing that makes a series synthetic is where its numbers came from, and a
    /// fixture renamed tomorrow is still a fixture. A source that does not resolve in
    /// the dimension reads as **not** derived: an unrecognised source is a finding to
    /// be graded, not a fixture to be set aside.
    synthetic: bool,
}

/// The label for a bar period. Anything off the known grid is printed as its own
/// second count rather than rounded into a neighbour — an unrecognised period is a
/// finding, not a 1m bar.
fn timeframe_label(period_secs: i64) -> String {
    match period_secs {
        60 => "1m".into(),
        300 => "5m".into(),
        900 => "15m".into(),
        3_600 => "1h".into(),
        14_400 => "4h".into(),
        86_400 => "1d".into(),
        n => format!("{n}s"),
    }
}

fn main() -> Result<(), String> {
    // LEFT JOIN, not INNER: an instrument with no row in the symbol dimension must
    // still appear, under its numeric id. An INNER JOIN would drop it, and a series
    // that vanishes from a census reads exactly like a series that has no problems.
    let inventory = query(&format!(
        "SELECT b.instrument_id, b.venue_id, b.period_secs, count() AS bars, \
                any(d.symbol) AS symbol, any(v.name) AS venue, \
                min(s.kind = 'derived') AS all_derived \
         FROM trading.market_bar AS b \
         LEFT JOIN trading.instrument_symbol_dim AS d \
                ON d.instrument_id = b.instrument_id AND d.venue_id = b.venue_id \
         LEFT JOIN trading.venue_dim AS v ON v.venue_id = b.venue_id \
         LEFT JOIN trading.source_dim AS s ON s.source_id = b.source_id \
         GROUP BY b.instrument_id, b.venue_id, b.period_secs \
         ORDER BY bars DESC {TSV}"
    ))?;

    let series: Vec<Series> = inventory
        .lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() < 7 {
                return None;
            }
            let instrument_id: i64 = f[0].parse().ok()?;
            let venue_id: i32 = f[1].parse().ok()?;
            let period_secs: i64 = f[2].parse().ok()?;
            let name = |s: &str, fallback: String| {
                if s.is_empty() {
                    fallback
                } else {
                    s.to_string()
                }
            };
            Some(Series {
                instrument_id,
                venue_id,
                period_secs,
                instrument: name(f[4], format!("instrument#{instrument_id}")),
                venue: name(f[5], format!("venue#{venue_id}")),
                timeframe: timeframe_label(period_secs),
                bars: f[3].parse().unwrap_or(0),
                synthetic: f[6].trim() == "1",
            })
        })
        .collect();

    let cfg = QcConfig::default();
    println!("config version {}\n", cfg.version());
    println!(
        "{:<28} {:<10} {:<5} {:>8}  {:<5} {:<4} first reason",
        "instrument", "venue", "tf", "bars", "grade", "kind"
    );
    let rule: String = std::iter::repeat_n('-', 124).collect();
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
    let mut synthetic_tally: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut d_reasons: Vec<String> = Vec::new();

    for s in &series {
        let bars = load(s.instrument_id, s.venue_id, s.period_secs)?;

        // The second venue, if one covers the same instrument and timeframe.
        let secondary = by_key
            .get(&(s.instrument.clone(), s.timeframe.clone()))
            .and_then(|v| v.iter().find(|o| o.venue != s.venue))
            .map(|o| load(o.instrument_id, o.venue_id, o.period_secs))
            .transpose()?;

        // Expected bars come from the census in production. Here the window is
        // taken from the series itself, which measures *internal* gaps and not
        // absence at the ends — stated rather than glossed, because it makes
        // these coverage numbers optimistic.
        let period = s.period_secs;
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
        if s.synthetic {
            *synthetic_tally.entry(grade.as_str()).or_default() += 1;
        } else {
            *tally.entry(grade.as_str()).or_default() += 1;
        }

        let first = report.graded.reasons.first().cloned().unwrap_or_default();
        println!(
            "{:<28} {:<10} {:<5} {:>8}  {:<5} {:<4} {}",
            s.instrument,
            s.venue,
            s.timeframe,
            s.bars,
            grade.as_str(),
            if s.synthetic { "SYN" } else { "live" },
            first.chars().take(58).collect::<String>()
        );

        if grade == Grade::D && !s.synthetic {
            d_reasons.push(format!(
                "{}/{}/{}: {}",
                s.instrument, s.venue, s.timeframe, first
            ));
        }
    }

    // The blast radius is a claim about the platform's *market data*, so a generated
    // series may not be counted into it. `market_bar` serves SYN-PLANTED-AR1-424242
    // — 5,000 1h bars from the `synthetic` source — beside real venue feeds, and it
    // is the longest-spanning series in the table. Folding it into this denominator
    // would let a fixture's grade move a number that is read as a statement about
    // vendor data. It is graded and printed, on its own line, and kept out of the
    // total.
    let live_count = series.iter().filter(|s| !s.synthetic).count();
    let synthetic_count = series.len() - live_count;

    println!("\n── blast radius (live venue series only) ──");
    for g in ["A", "B", "C", "D"] {
        println!("  {g}: {}", tally.get(g).copied().unwrap_or(0));
    }
    let refused = tally.get("D").copied().unwrap_or(0);
    let waiver = tally.get("C").copied().unwrap_or(0);
    println!(
        "\n{refused} series would be REFUSED outright and {waiver} would need a human waiver \
         if P-2.3 were switched on today, out of {live_count} live venue series."
    );
    if synthetic_count > 0 {
        let grades: Vec<String> = ["A", "B", "C", "D"]
            .iter()
            .filter_map(|g| {
                let n = synthetic_tally.get(g).copied().unwrap_or(0);
                (n > 0).then(|| format!("{n}{g}"))
            })
            .collect();
        println!(
            "\n{synthetic_count} generated series excluded from that total ({}). \
             These are fixtures written by a `derived` source, not vendor data; their \
             grades describe the generator and must not be quoted as data quality.",
            grades.join(", ")
        );
    }
    if !d_reasons.is_empty() {
        println!("\nWhy the D grades:");
        for r in &d_reasons {
            println!("  - {r}");
        }
    }
    Ok(())
}

/// Every bar of one series, oldest first, revisions included.
///
/// `QcBar::event_ns` is documented as the bar's **open**, and `market_bar.event_time`
/// is the open, so the two map straight across. The v2 table stored the bar's *close*
/// in its `event_time`, so the old census was quietly feeding a close time into a
/// field that means open — one more thing the swap puts right.
///
/// Revisions are deliberately not collapsed: `QcBar::revised` is how the battery sees
/// that a vendor restated a bar, and a `FINAL` or `argMax` read would hide exactly
/// that. This is the property `market_bars_v2` could not offer — with `available_time`
/// outside its sorting key the superseded row is already gone. It is not a small
/// difference. Measured 2026-09-22, BTC-USD/kraken/1m holds 18 event times carrying
/// more than one row — 73 restatements in all, one bar of it restated 54 times — and
/// every one of those groups has a distinct `knowledge_time` *and* a distinct
/// `revision_seq`, so a point-in-time read resolves each to exactly one bar. The v2
/// table showed that same series as revision-free.
fn load(instrument_id: i64, venue_id: i32, period_secs: i64) -> Result<Vec<QcBar>, String> {
    let sql = format!(
        "SELECT toUnixTimestamp64Nano(event_time), \
                toUnixTimestamp64Nano(knowledge_time), \
                toFloat64(open), toFloat64(high), toFloat64(low), toFloat64(close), \
                toFloat64(volume), revision_seq \
         FROM trading.market_bar \
         WHERE instrument_id = {instrument_id} AND venue_id = {venue_id} \
           AND period_secs = {period_secs} \
         ORDER BY event_time, knowledge_time, revision_seq FORMAT TSV"
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
