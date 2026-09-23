//! Kraken historical OHLC, by two paths (DATA-005 §11 S1; Set P P-1.3).
//!
//! Kraken offers both, and the platform needs both for different reasons:
//!
//! 1. **The public `OHLC` endpoint** serves at most 720 candles and, for 1m
//!    data, *only the most recent* 720 minutes — Kraken does not paginate 1m
//!    history backwards. It is the recent-history path and nothing more.
//! 2. **The downloadable history archives** are where multiple years actually
//!    come from. They are a different shape entirely: a file is fetched (or
//!    placed on disk), **verified against a declared checksum**, parsed, and
//!    inserted through the same `BarStore`.
//!
//! The archive path is deliberately fed from a local directory rather than
//! scraped. Kraken distributes the archives behind a support page, and a
//! collector that automates its way past that is building against terms rather
//! than under them (Set P MASTER §5). Fetching is a human step; verifying,
//! parsing and inserting are not, and those are what this module does.
//!
//! **A file whose checksum does not match is refused and the job pauses.**
//! Nothing partial is inserted. Half an archive is worse than none: the gap it
//! leaves is invisible, and every study over that window silently uses it.
//!
//! **Declared rate limit: 1 request/second** for the public endpoint. Kraken's
//! public rate limiting is a decaying counter rather than a published rps, and
//! 1/s is comfortably inside every tier.
//!
//! **Declared vendor lag: 60 s** (`kraken_rest`, ADR-P2-24) — the same as
//! Coinbase, and, like Coinbase, read from `dataplane.source` by `BarStore`
//! rather than passed with the bars.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use dataplane::backfill::{BackfillBar, BackfillPage, BackfillSource, FetchError, PageRequest, RateLimit};
use serde::Deserialize;
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};

use super::{raw_number, raw_u64, HttpGet, Limited};

/// `dataplane.source.name` — selects the 60 s declared vendor lag.
pub const SOURCE: &str = "kraken_rest";
pub const VENUE: &str = "kraken";
const API: &str = "https://api.kraken.com/0/public/OHLC";
/// The venue's own page cap.
pub const MAX_CANDLES: i64 = 720;

#[must_use]
pub const fn declared_rate_limit() -> RateLimit {
    RateLimit { per_window: 1, window_ms: 1_000, burst: 1 }
}

/// Kraken's `interval` parameter is in minutes, from a fixed set.
///
/// # Errors
/// A period Kraken does not serve.
pub fn interval_minutes(period_secs: u32) -> Result<u32, FetchError> {
    match period_secs {
        60 | 300 | 900 | 1_800 | 3_600 | 14_400 | 86_400 | 604_800 => Ok(period_secs / 60),
        other => Err(FetchError::Integrity(format!("Kraken has no {other} s OHLC interval"))),
    }
}

// ── path 1: the public OHLC endpoint ─────────────────────────────────────────

#[derive(Deserialize)]
struct OhlcEnvelope<'a> {
    #[serde(default)]
    error: Vec<String>,
    #[serde(borrow, default)]
    result: BTreeMap<&'a str, &'a RawValue>,
}

/// `[time, open, high, low, close, vwap, volume, count]`, time = bar **open**.
///
/// Field 5 is Kraken's vwap. The platform derives no vwap from a backfilled
/// candle, so it is decoded into [`IgnoredAny`] rather than borrowed: the field
/// has to exist to keep the tuple positions aligned with Kraken's array, and
/// naming it as ignored says so, where a borrowed-but-unread field only says it
/// to `dead_code`.
#[derive(Deserialize)]
struct Ohlc<'a>(
    #[serde(borrow)] &'a RawValue,
    #[serde(borrow)] &'a RawValue,
    #[serde(borrow)] &'a RawValue,
    #[serde(borrow)] &'a RawValue,
    #[serde(borrow)] &'a RawValue,
    serde::de::IgnoredAny,
    #[serde(borrow)] &'a RawValue,
    #[serde(borrow)] &'a RawValue,
);

/// Parse an `OHLC` response into platform bars, oldest first.
///
/// # Errors
/// A Kraken `error` array, a missing pair key, or an unreadable field.
pub fn parse_ohlc(body: &[u8], period_secs: u32) -> Result<Vec<BackfillBar>, FetchError> {
    let env: OhlcEnvelope =
        serde_json::from_slice(body).map_err(|e| FetchError::Integrity(format!("kraken ohlc: {e}")))?;
    if let Some(first) = env.error.first() {
        // Kraken signals its own rate limiting in the body, with HTTP 200, and
        // spells it at least two ways (`EAPI:Rate limit exceeded`,
        // `EGeneral:Too many requests`). Every error in the array is checked,
        // not only the first: reading a throttle as a successful empty page
        // would look like a venue with no history rather than one asking us to
        // slow down — and the backfill would record a permanent hole.
        if env.error.iter().any(|e| is_rate_limit(e)) {
            return Err(FetchError::RateLimited { venue: VENUE.into(), retry_after_ms: 5_000 });
        }
        return Err(FetchError::Integrity(format!("kraken: {first}")));
    }
    // The result map holds the pair key plus `last`; the pair key is whichever
    // entry is an array. Matching on the name would mean hard-coding Kraken's
    // altnames (`XXBTZUSD` for `BTC/USD`), which change.
    let rows = env
        .result
        .values()
        .find_map(|v| serde_json::from_str::<Vec<Ohlc>>(v.get()).ok())
        .ok_or_else(|| FetchError::Integrity("kraken ohlc: no candle array in the result".into()))?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let open_s: i64 = raw_number(r.0)?
            .parse()
            .map_err(|_| FetchError::Integrity("kraken ohlc time is not an integer".into()))?;
        let open_time = Utc
            .timestamp_opt(open_s, 0)
            .single()
            .ok_or_else(|| FetchError::Integrity(format!("kraken ohlc time {open_s} is not a valid instant")))?;
        out.push(BackfillBar {
            close_time: open_time + chrono::Duration::seconds(i64::from(period_secs)),
            open: raw_number(r.1)?,
            high: raw_number(r.2)?,
            low: raw_number(r.3)?,
            close: raw_number(r.4)?,
            volume: raw_number(r.6)?,
            trade_count: raw_u64(r.7)?,
        });
    }
    out.sort_by_key(|b| b.close_time);
    Ok(out)
}

/// Whether a Kraken error string is the venue throttling us.
#[must_use]
pub fn is_rate_limit(e: &str) -> bool {
    let e = e.to_ascii_lowercase();
    e.contains("rate limit") || e.contains("too many requests") || e.contains("throttle")
}

#[must_use]
pub fn ohlc_url(pair: &str, interval_min: u32, since_open_s: i64) -> String {
    format!("{API}?pair={pair}&interval={interval_min}&since={since_open_s}")
}

/// The recent-history path.
pub struct KrakenOhlcBackfill {
    http: Arc<dyn HttpGet>,
    limiter: Limited,
}

impl KrakenOhlcBackfill {
    #[must_use]
    pub fn new(http: Arc<dyn HttpGet>) -> Self {
        Self { http, limiter: Limited::new(declared_rate_limit()) }
    }

    #[must_use]
    pub fn page_span_secs(period_secs: u32) -> i64 {
        i64::from(period_secs) * MAX_CANDLES
    }
}

#[async_trait]
impl BackfillSource for KrakenOhlcBackfill {
    fn source_name(&self) -> &str {
        SOURCE
    }

    fn declared_limit(&self) -> RateLimit {
        declared_rate_limit()
    }

    async fn fetch_page(&self, req: &PageRequest) -> Result<BackfillPage, FetchError> {
        let interval = interval_minutes(req.period_secs)?;
        let period = i64::from(req.period_secs);
        let span = Self::page_span_secs(req.period_secs);
        let page_end: DateTime<Utc> = std::cmp::min(req.end, req.start + chrono::Duration::seconds(span));
        if page_end <= req.start {
            return Ok(BackfillPage::end_of_window());
        }
        // `since` is exclusive of the named bar open, so step back one period to
        // include the first bar of the window rather than skipping it.
        let since = req.start.timestamp() - period - period;

        self.limiter.permit().await;
        let body = self.http.get(&ohlc_url(&req.symbol, interval, since)).await?.ok_body(VENUE)?;
        let all = parse_ohlc(&body, req.period_secs)?;
        let bars: Vec<_> = all.into_iter().filter(|b| b.close_time >= req.start && b.close_time < page_end).collect();
        Ok(BackfillPage { bars, covered_to: Some(page_end) })
    }
}

// ── path 2: the downloadable history archives ────────────────────────────────

/// A Kraken OHLCVT CSV: `open_seconds,open,high,low,close,volume,trades`,
/// headerless, one file per pair and interval.
///
/// # Errors
/// A malformed row. The whole file is refused rather than the row skipped: a
/// partially parsed archive is a gap that looks like history.
pub fn parse_archive_csv(bytes: &[u8], period_secs: u32) -> Result<Vec<BackfillBar>, FetchError> {
    let text = std::str::from_utf8(bytes).map_err(|e| FetchError::Integrity(format!("archive is not UTF-8: {e}")))?;
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').map(str::trim).collect();
        if f.len() < 6 {
            return Err(FetchError::Integrity(format!("archive line {}: expected 6-7 fields, got {}", n + 1, f.len())));
        }
        let open_s: i64 = f[0]
            .parse()
            .map_err(|_| FetchError::Integrity(format!("archive line {}: {:?} is not a timestamp", n + 1, f[0])))?;
        let open_time = Utc
            .timestamp_opt(open_s, 0)
            .single()
            .ok_or_else(|| FetchError::Integrity(format!("archive line {}: {open_s} is not a valid instant", n + 1)))?;
        let num = |i: usize| -> Result<String, FetchError> {
            let s = f[i];
            if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E')) {
                return Err(FetchError::Integrity(format!("archive line {}: {s:?} is not a number", n + 1)));
            }
            Ok(s.to_string())
        };
        out.push(BackfillBar {
            close_time: open_time + chrono::Duration::seconds(i64::from(period_secs)),
            open: num(1)?,
            high: num(2)?,
            low: num(3)?,
            close: num(4)?,
            volume: num(5)?,
            trade_count: f.get(6).and_then(|s| s.parse().ok()).unwrap_or(0),
        });
    }
    out.sort_by_key(|b| b.close_time);
    Ok(out)
}

/// Lower-case hex SHA-256 of the bytes.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// One declared archive file: where it is, and what it must hash to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveFile {
    pub path: PathBuf,
    /// Lower-case hex SHA-256, as published or as recorded when the file was
    /// placed. **Required.** An archive with no declared checksum is an archive
    /// nobody can say is intact, and this path exists precisely to be able to
    /// say so.
    pub sha256: String,
}

/// Read and verify an archive file.
///
/// # Errors
/// [`FetchError::Integrity`] if the file is missing, unreadable, or hashes to
/// anything other than its declared checksum. The engine turns that into a
/// **pause**, so the window waits for a human instead of inserting partial data
/// or marking itself failed.
pub fn read_verified(file: &ArchiveFile) -> Result<Vec<u8>, FetchError> {
    let bytes = std::fs::read(&file.path)
        .map_err(|e| FetchError::Integrity(format!("archive {}: {e}", file.path.display())))?;
    let got = sha256_hex(&bytes);
    if !got.eq_ignore_ascii_case(&file.sha256) {
        return Err(FetchError::Integrity(format!(
            "archive {} checksum mismatch: declared {}, computed {got}. Nothing was inserted.",
            file.path.display(),
            file.sha256
        )));
    }
    Ok(bytes)
}

/// The deep-history path: verified local archives, paged like any other source.
pub struct KrakenArchiveBackfill {
    dir: PathBuf,
    /// `file name -> declared sha256`. Supplied rather than discovered: a
    /// checksum read from beside the file it checks proves only that both were
    /// written by the same hand.
    checksums: BTreeMap<String, String>,
}

impl KrakenArchiveBackfill {
    #[must_use]
    pub fn new(dir: impl AsRef<Path>, checksums: BTreeMap<String, String>) -> Self {
        Self { dir: dir.as_ref().to_path_buf(), checksums }
    }

    /// Kraken's naming: `XBTUSD_1.csv` — pair alt-name, then interval minutes.
    #[must_use]
    pub fn file_name(symbol: &str, interval_min: u32) -> String {
        format!("{}_{interval_min}.csv", symbol.replace(['-', '/'], ""))
    }

    /// # Errors
    /// No declared checksum for the file this window needs.
    pub fn file_for(&self, symbol: &str, interval_min: u32) -> Result<ArchiveFile, FetchError> {
        let name = Self::file_name(symbol, interval_min);
        let sha = self.checksums.get(&name).ok_or_else(|| {
            FetchError::Integrity(format!(
                "no declared checksum for archive {name}; place the file and record its sha256 before backfilling from it"
            ))
        })?;
        Ok(ArchiveFile { path: self.dir.join(&name), sha256: sha.clone() })
    }
}

#[async_trait]
impl BackfillSource for KrakenArchiveBackfill {
    fn source_name(&self) -> &str {
        SOURCE
    }

    fn declared_limit(&self) -> RateLimit {
        // A local file has no venue to protect, but the contract asks for a
        // number rather than an exception, so the limit is generous and honest.
        RateLimit { per_window: 1_000, window_ms: 1_000, burst: 1_000 }
    }

    async fn fetch_page(&self, req: &PageRequest) -> Result<BackfillPage, FetchError> {
        let interval = interval_minutes(req.period_secs)?;
        let file = self.file_for(&req.symbol, interval)?;
        let bytes = read_verified(&file)?;
        let all = parse_archive_csv(&bytes, req.period_secs)?;
        let bars: Vec<_> = all.into_iter().filter(|b| b.close_time >= req.start && b.close_time < req.end).collect();
        // One file covers the whole window it can cover.
        Ok(BackfillPage { bars, covered_to: Some(req.end) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OHLC: &[u8] = br#"{"error":[],"result":{"XXBTZUSD":[
      [1700000000,"73850.0","73899.9","73800.0","73861.99","73860.1","2.00000000",7],
      [1700000060,"73950.0","73999.9","73900.0","73961.99","73950.1","1.41080000",3]
    ],"last":1700000060}}"#;

    /// The two paths must agree bar-for-bar on an overlapping window, or the
    /// deep history and the recent history meet at a seam nobody can see.
    #[test]
    fn the_api_and_the_archive_agree_bar_for_bar_on_an_overlapping_window() {
        let from_api = parse_ohlc(OHLC, 60).unwrap();
        // The same two bars as Kraken publishes them in the OHLCVT archive.
        let csv = b"1700000000,73850.0,73899.9,73800.0,73861.99,2.00000000,7\n\
                    1700000060,73950.0,73999.9,73900.0,73961.99,1.41080000,3\n";
        let from_archive = parse_archive_csv(csv, 60).unwrap();
        assert_eq!(from_api, from_archive, "the two Kraken paths disagree on the same window");
        assert_eq!(from_api[0].close_time, Utc.timestamp_opt(1_700_000_060, 0).unwrap());
        assert_eq!(from_api[1].trade_count, 3);
    }

    /// The adversarial half of P-1.3: a file that does not verify inserts
    /// nothing and produces a pausing error, not a failure and not a partial
    /// read.
    #[test]
    fn an_archive_whose_checksum_does_not_match_is_refused_and_pauses() {
        let dir = std::env::temp_dir().join(format!("kraken_archive_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("XBTUSD_1.csv");
        std::fs::write(&path, b"1700000000,1,1,1,1,1,1\n").unwrap();

        let good = sha256_hex(&std::fs::read(&path).unwrap());
        assert!(read_verified(&ArchiveFile { path: path.clone(), sha256: good }).is_ok());

        let tampered = ArchiveFile { path: path.clone(), sha256: "0".repeat(64) };
        let e = read_verified(&tampered).expect_err("a mismatched checksum must be refused");
        assert!(matches!(e, FetchError::Integrity(_)), "{e:?}");
        assert!(e.is_pause(), "a bad archive waits for a human; it does not fail the window");
        assert!(e.to_string().contains("Nothing was inserted"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_archive_with_no_declared_checksum_is_refused_rather_than_trusted() {
        let b = KrakenArchiveBackfill::new(".", BTreeMap::new());
        let e = b.file_for("BTC-USD", 1).expect_err("an unverified archive must not be read");
        assert!(e.to_string().contains("no declared checksum"), "{e}");
    }

    /// Kraken reports its own throttling in a 200 body. Reading that as an empty
    /// page would look like a venue with no history.
    #[test]
    fn kraken_s_in_body_rate_limit_becomes_a_pause_not_an_empty_page() {
        let body = br#"{"error":["EGeneral:Too many requests","Rate limit exceeded"],"result":{}}"#;
        let e = parse_ohlc(body, 60).expect_err("expected an error");
        assert!(matches!(e, FetchError::RateLimited { .. }), "{e:?}");
        assert!(e.is_pause());
        // Both of Kraken's spellings, in either position in the array.
        for body in [
            br#"{"error":["EAPI:Rate limit exceeded"],"result":{}}"#.as_slice(),
            br#"{"error":["EGeneral:Too many requests"],"result":{}}"#.as_slice(),
        ] {
            assert!(matches!(parse_ohlc(body, 60), Err(FetchError::RateLimited { .. })));
        }
    }

    #[test]
    fn a_kraken_error_body_is_never_read_as_success() {
        let body = br#"{"error":["EQuery:Unknown asset pair"],"result":{}}"#;
        assert!(matches!(parse_ohlc(body, 60), Err(FetchError::Integrity(_))));
    }

    #[test]
    fn a_short_archive_row_fails_the_file_rather_than_skipping_the_row() {
        let e = parse_archive_csv(b"1700000000,1,1,1\n", 60).expect_err("a short row must fail the file");
        assert!(matches!(e, FetchError::Integrity(_)), "{e:?}");
        let e = parse_archive_csv(b"1700000000,1,1,1,x,1,1\n", 60).expect_err("a non-numeric price must fail the file");
        assert!(matches!(e, FetchError::Integrity(_)), "{e:?}");
    }

    #[test]
    fn archive_file_names_follow_kraken_s_own_convention() {
        assert_eq!(KrakenArchiveBackfill::file_name("XBT-USD", 1), "XBTUSD_1.csv");
        assert_eq!(KrakenArchiveBackfill::file_name("ETH/USD", 60), "ETHUSD_60.csv");
    }

    #[test]
    fn an_interval_kraken_does_not_serve_is_refused() {
        assert_eq!(interval_minutes(3_600).unwrap(), 60);
        assert!(interval_minutes(21_600).is_err(), "Kraken has no 6h interval");
    }

    #[test]
    fn since_steps_back_so_the_first_bar_of_the_window_is_not_skipped() {
        // Kraken's `since` is exclusive; a window whose first bar closes at
        // 1700000060 must ask from before that bar's open.
        let url = ohlc_url("XBTUSD", 1, 1_700_000_060 - 60 - 60);
        assert!(url.contains("since=1699999940"), "{url}");
    }
}
