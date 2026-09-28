//! P-1.2's first acceptance clause: **a 30-day BTC-USD 1m window lands with no
//! gaps the venue does not itself have.**
//!
//! Every other test of the Coinbase adapter exercises one function on one page.
//! This one drives the adapter the way a backfill job does — page after page,
//! following `covered_to` — against a fixture venue that holds a full 30 days of
//! minutes with a hole deliberately punched in it. That is the only way to catch
//! the class of defect that matters here: a page-span arithmetic error, a cursor
//! that advances by the wrong amount, or a URL whose `start`/`end` are a period
//! out. None of those are visible in a single-page test, and all of them produce
//! a series with holes that no later stage can distinguish from a venue outage.
//!
//! The fixture answers in the venue's own shape: `[time, low, high, open, close,
//! volume]` with `time` the bar **open** in epoch seconds, newest first, capped at
//! the venue's 300 candles, with `start`/`end` read as inclusive bounds on the
//! open. It also counts requests, so the limiter can be shown to have granted
//! exactly one permit per request rather than the adapter having found a path that
//! skips it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use collectors::backfill::coinbase::{CoinbaseBackfill, MAX_CANDLES, SOURCE};
use collectors::backfill::{HttpGet, HttpResponse};
use dataplane::backfill::{BackfillSource, FetchError, PageRequest};

const PERIOD: i64 = 60;
const DAYS: i64 = 30;
const MINUTES: i64 = DAYS * 24 * 60;

/// The minute (as an index from the window start) the venue itself is missing,
/// and how many minutes it is missing for.
const GAP_AT: i64 = 10_000;
const GAP_LEN: i64 = 7;

fn window_start() -> DateTime<Utc> {
    // A round hour, so bucket arithmetic errors are not hidden by an offset that
    // happens to cancel.
    Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap()
}

/// A venue holding `MINUTES` minutes of candles from `window_start`, less a gap.
struct FixtureVenue {
    /// bar OPEN seconds → the candle's JSON body fragment.
    candles: BTreeMap<i64, String>,
    requests: AtomicU64,
    /// Every `(start, end)` pair asked for, in order.
    asked: Mutex<Vec<(i64, i64)>>,
}

impl FixtureVenue {
    fn new() -> Self {
        let base = window_start().timestamp();
        let mut candles = BTreeMap::new();
        for i in 0..MINUTES {
            if (GAP_AT..GAP_AT + GAP_LEN).contains(&i) {
                continue; // the hole the venue itself has
            }
            // The bar CLOSING at base + (i+1)*60 OPENS at base + i*60.
            let open_s = base + i * PERIOD;
            // Prices that move, and with more fractional digits than an f64 keeps,
            // so a float round-trip anywhere in the path would show up.
            let px = 70_000 + (i * 13) % 977;
            candles.insert(
                open_s,
                format!(
                    "[{open_s},\"{px}.111111111111111111\",\"{}.999999999999999999\",\
                     \"{px}.555555555555555555\",\"{px}.777777777777777777\",\"{}.00000001\"]",
                    px + 1,
                    i + 1
                ),
            );
        }
        Self {
            candles,
            requests: AtomicU64::new(0),
            asked: Mutex::new(Vec::new()),
        }
    }

    fn param(url: &str, key: &str) -> i64 {
        url.split(['?', '&'])
            .find_map(|kv| kv.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| panic!("no {key} in {url}"))
            .parse()
            .unwrap_or_else(|_| panic!("{key} is not an integer in {url}"))
    }
}

#[async_trait]
impl HttpGet for FixtureVenue {
    async fn get(&self, url: &str) -> Result<HttpResponse, FetchError> {
        assert!(url.contains("granularity=60"), "{url}");
        assert!(
            url.contains("/products/BTC-USD/candles"),
            "the adapter must ask for the product it was given: {url}"
        );
        let start = Self::param(url, "start");
        let end = Self::param(url, "end");
        self.requests.fetch_add(1, Ordering::Relaxed);
        self.asked.lock().unwrap().push((start, end));

        // The venue's own bounds are inclusive on the bar open, newest first, and
        // capped at 300 candles.
        let mut rows: Vec<&String> = self
            .candles
            .range(start..=end)
            .map(|(_, body)| body)
            .collect();
        assert!(
            rows.len() <= usize::try_from(MAX_CANDLES).unwrap(),
            "the adapter asked for {} candles, past the venue's {MAX_CANDLES} cap",
            rows.len()
        );
        rows.reverse();
        Ok(HttpResponse {
            status: 200,
            retry_after_ms: None,
            body: format!(
                "[{}]",
                rows.iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            )
            .into_bytes(),
        })
    }
}

/// Walk the window the way the backfill engine does: ask for a page, follow
/// `covered_to`, stop when the venue says there is nothing further.
async fn walk(
    adapter: &CoinbaseBackfill,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<(DateTime<Utc>, String)> {
    let mut cursor = start;
    let mut delivered = Vec::new();
    let mut pages = 0_u32;
    while cursor < end {
        let page = adapter
            .fetch_page(&PageRequest {
                venue: "coinbase".into(),
                symbol: "BTC-USD".into(),
                timeframe: "1m".into(),
                period_secs: 60,
                start: cursor,
                end,
            })
            .await
            .expect("the fixture venue never refuses");
        for b in page.bars {
            delivered.push((b.close_time, b.close));
        }
        let Some(next) = page.covered_to else { break };
        assert!(
            next > cursor,
            "a page that does not advance the cursor is an infinite loop"
        );
        cursor = next;
        pages += 1;
        assert!(pages < 1_000, "the walk is not terminating");
    }
    delivered
}

/// P-1.2, clause 1. Thirty days of minutes, paged through the real adapter, land
/// with exactly the venue's own gap and no other.
#[tokio::test]
async fn a_thirty_day_one_minute_window_lands_with_no_gaps_the_venue_does_not_itself_have() {
    let venue = Arc::new(FixtureVenue::new());
    let adapter = CoinbaseBackfill::new(venue.clone());
    let start = window_start();
    let end = start + Duration::minutes(MINUTES);

    let delivered = walk(&adapter, start, end).await;

    // What the venue actually holds, in platform close time.
    let expected: BTreeSet<DateTime<Utc>> = (0..MINUTES)
        .filter(|i| !(GAP_AT..GAP_AT + GAP_LEN).contains(i))
        .map(|i| start + Duration::minutes(i + 1))
        .collect();
    let got: BTreeSet<DateTime<Utc>> = delivered.iter().map(|(t, _)| *t).collect();

    assert_eq!(
        got.len(),
        expected.len(),
        "{} minutes delivered, {} held by the venue",
        got.len(),
        expected.len()
    );
    assert_eq!(
        got, expected,
        "the delivered minutes are exactly the held ones"
    );

    // And the only hole is the venue's, located and measured rather than merely
    // counted: a shifted series has the right count and the wrong dates.
    let mut holes: Vec<(DateTime<Utc>, i64)> = Vec::new();
    let ordered: Vec<DateTime<Utc>> = got.iter().copied().collect();
    for pair in ordered.windows(2) {
        let step = (pair[1] - pair[0]).num_minutes();
        if step > 1 {
            holes.push((pair[0], step - 1));
        }
    }
    assert_eq!(
        holes,
        vec![(start + Duration::minutes(GAP_AT), GAP_LEN)],
        "exactly one hole, at the minute the fixture venue is missing, of its length"
    );
    assert_eq!(*ordered.first().unwrap(), start + Duration::minutes(1));
    assert_eq!(*ordered.last().unwrap(), end);
}

/// The paging arithmetic, asserted directly rather than inferred from coverage:
/// no request ever exceeds the venue's 300-candle cap, the walk takes the number
/// of pages that span implies, and the limiter granted exactly one permit per
/// request — so there is no path through `fetch_page` that reaches the network
/// without asking it.
#[tokio::test]
async fn the_window_costs_exactly_the_pages_the_venues_cap_implies() {
    let venue = Arc::new(FixtureVenue::new());
    let adapter = CoinbaseBackfill::new(venue.clone());
    let start = window_start();
    walk(&adapter, start, start + Duration::minutes(MINUTES)).await;

    // 299 minutes per page, because the venue's bounds are inclusive on the open.
    let per_page = CoinbaseBackfill::page_span_secs(60) / PERIOD;
    assert_eq!(per_page, MAX_CANDLES - 1);
    let expected_pages = u64::try_from((MINUTES + per_page - 1) / per_page).unwrap();
    assert_eq!(expected_pages, 145);
    assert_eq!(
        venue.requests.load(Ordering::Relaxed),
        expected_pages,
        "more pages than this means the cursor is advancing short of what each page covered"
    );
    assert_eq!(
        adapter.limiter_granted(),
        expected_pages,
        "one permit per request, with no path that skips the limiter"
    );
}

/// The page boundaries, in the venue's own units.
///
/// Each page asks for opens `[close_start − 60, close_end − 60]` inclusive, so a
/// page sized to the exact 300-candle cap asks for 301 — which is what the adapter
/// did before this test existed, and what the fixture venue refuses. Sized one
/// period short, a page asks for exactly 300 and overlaps its successor by exactly
/// one candle.
///
/// That one-candle overlap is the **safe** side of an ambiguity rather than an
/// oversight: whether `end` is inclusive is not verifiable here without calling the
/// live venue, and being wrong in the other direction drops one bar per page — 145
/// invented holes across a 30-day window, indistinguishable from venue outages and
/// unrecoverable once the cursor has passed them. A re-fetched candle resolves to
/// `unchanged` at the sink. What must be bounded is the overlap: **at most one
/// candle**, so neither reading leaves a bar out.
#[tokio::test]
async fn page_boundaries_overlap_by_at_most_one_candle_and_never_leave_one_out() {
    let venue = Arc::new(FixtureVenue::new());
    let adapter = CoinbaseBackfill::new(venue.clone());
    let start = window_start();
    walk(&adapter, start, start + Duration::minutes(MINUTES)).await;

    let asked = venue.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 145);
    // The first page begins at the open of the bar closing at the window start.
    assert_eq!(asked[0].0, start.timestamp() - PERIOD);
    for (i, (s, e)) in asked.iter().enumerate() {
        let opens = (e - s) / PERIOD + 1;
        assert!(
            opens <= MAX_CANDLES,
            "page {i} asks for {opens} candle opens, past the {MAX_CANDLES} cap"
        );
    }
    for pair in asked.windows(2) {
        let (_, prev_end) = pair[0];
        let (next_start, _) = pair[1];
        let overlap = (prev_end - next_start) / PERIOD + 1;
        assert_eq!(
            overlap, 1,
            "consecutive pages must overlap by exactly one candle: leaving one out is a \
             fabricated gap, and overlapping by more is wasted requests"
        );
    }
}

/// Prices survive the whole path with the digits the venue sent. An `f64`
/// anywhere between the fixture body and `BackfillBar` would round these, and a
/// rounded price is a wrong price that validates (SC-1, INV-05).
#[tokio::test]
async fn prices_keep_every_digit_across_a_multi_page_walk() {
    let venue = Arc::new(FixtureVenue::new());
    let adapter = CoinbaseBackfill::new(venue.clone());
    let start = window_start();
    // Two pages' worth, so the assertion covers a boundary.
    let delivered = walk(&adapter, start, start + Duration::minutes(600)).await;

    let sample = &delivered[0].1;
    assert_eq!(sample, "70000.777777777777777777");
    assert_ne!(
        sample.parse::<f64>().unwrap().to_string(),
        *sample,
        "the fixture price is one f64 cannot hold, which is what makes this test mean something"
    );
    assert!(
        delivered
            .iter()
            .all(|(_, close)| close.len() == sample.len()),
        "every close kept its eighteen fractional digits"
    );
}

/// The adapter names the source whose declared 60 s lag stamps every row, and it
/// does not stamp the lag itself. A lag carried alongside the bars is a lag that
/// can disagree with `dataplane.source` (ADR-P2-24).
#[test]
fn the_adapter_names_its_source_and_stamps_no_lag_of_its_own() {
    let venue = Arc::new(FixtureVenue::new());
    let adapter = CoinbaseBackfill::new(venue);
    assert_eq!(adapter.source_name(), SOURCE);
    assert_eq!(adapter.source_name(), "coinbase_rest");
    assert_eq!(
        dataplane::source::SourceDecl::coinbase_rest().declared_lag,
        std::time::Duration::from_secs(60)
    );
}
