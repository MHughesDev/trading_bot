//! Alpaca News API → `text_items` (DATA-005 §11 S2, §10; Set P, P-3.3).
//!
//! A news item's knowledge time is its **publication time**, not the moment the
//! collector happened to fetch it. Alpaca gives both (`created_at` and
//! `updated_at`), and the difference between them is what tells a correction from
//! an original: an item whose `updated_at` is later than its `created_at` is a
//! *revision*, and a revision is a new row with a higher `revision` number, never
//! an update of the row already stored ([`dataplane::text`]).
//!
//! ## Free-plan history depth — re-verified 2026-09-22
//!
//! `docs.alpaca.markets/docs/historical-news-data` states it plainly: "This API
//! provides historical news data dating back to **2015**", and "All news data is
//! currently provided directly by Benzinga". The archive depth is **2015** and is
//! not tiered — the paid plans buy request rate, not history. `/v1beta1/news`
//! accepts `start`/`end` over that range with a `page_token` cursor, so the
//! series is genuinely backfillable.
//!
//! That is the finding P-3.3 asks for, and it is the *good* outcome: news does
//! **not** join the FM-11 forward-collect argument the way funding rates and
//! option chains do. It is still worth collecting forward, because a corrected
//! item's original text is only observable if you were reading at the time — the
//! archive serves the current version of a story, so revision history before the
//! collector started is unrecoverable even though the text is not.
//!
//! **Not verified:** the per-plan request ceiling. That page states the depth and
//! the supplier but no rate, so [`VenueLimit::alpaca_market_data`] declares a
//! self-imposed budget and says so rather than quoting a number nobody published.

use chrono::{DateTime, Utc};
use dataplane::text::TextItem;
use serde::Deserialize;

use crate::ratelimit::{RateLimiter, VenueLimit};

/// Alpaca's data host for news.
pub const ALPACA_DATA_BASE: &str = "https://data.alpaca.markets";

/// The `dataplane.source.name` this collector writes under. The row itself is a
/// migration this track does not write — see the report.
pub const SOURCE_NAME: &str = "alpaca_news";

// No `SOURCE_ID` const here on purpose. `dataplane.source` ids 1–19 are already
// allocated (migrations 0046 and 0098) and no row for `alpaca_news` exists yet, so
// any number written here would be a guess that silently disagrees with whatever
// the migration author picks. [`to_text_items`] takes the id as an argument
// instead: the caller reads it from `dataplane.source` by name and a foreign key
// refuses a wrong one, where a const would sail through.

/// The earliest date the free-plan archive serves.
///
/// Alpaca documents the depth as "dating back to 2015" without naming a day, so
/// the first of that year is the conservative floor to request from. See the
/// module note for the verification.
pub const FREE_PLAN_HISTORY_START: &str = "2015-01-01";

#[derive(Debug, thiserror::Error)]
pub enum NewsError {
    #[error("malformed response: {0}")]
    Parse(String),
    /// A correction cannot predate what it corrects. Storing one would make an
    /// as-of read return the correction for an instant before it happened.
    #[error(
        "news item {id}: updated_at {updated} precedes created_at {created} — \
         a correction cannot predate the item it corrects"
    )]
    CorrectionPredatesOriginal {
        id: i64,
        created: DateTime<Utc>,
        updated: DateTime<Utc>,
    },
}

// ── Alpaca response shapes ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AlpacaNewsResponse {
    #[serde(default)]
    pub news: Vec<AlpacaNewsItem>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AlpacaNewsItem {
    pub id: i64,
    pub headline: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub symbols: Vec<String>,
    /// Publication time.
    pub created_at: DateTime<Utc>,
    /// Later than `created_at` only when the publisher corrected the item.
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

/// The stable identity of a story across its revisions.
#[must_use]
pub fn item_id(alpaca_id: i64) -> String {
    format!("alpaca:{alpaca_id}")
}

/// Turns one Alpaca item into the rows it implies: the original, plus one
/// revision row when the publisher has corrected it.
///
/// `source_id` is the `dataplane.source` row for this feed, read by name rather
/// than assumed; `ingested_time` is when the collector saw it, and
/// `declared_lag_ms` is the source's declared lag — 0 for the live stream,
/// non-zero for a delayed feed.
///
/// # Errors
/// [`NewsError::CorrectionPredatesOriginal`] for an item whose `updated_at`
/// precedes its `created_at`.
pub fn to_text_items(
    item: &AlpacaNewsItem,
    source_id: i32,
    ingested_time: DateTime<Utc>,
    declared_lag_ms: i64,
) -> Result<Vec<TextItem>, NewsError> {
    let id = item_id(item.id);
    let mut original = TextItem::first(
        id,
        source_id,
        item.id.to_string(),
        item.headline.clone(),
        item.created_at,
        ingested_time,
        declared_lag_ms,
    );
    original.body = item.content.clone().or_else(|| item.summary.clone());
    original.symbols = item.symbols.clone();

    let Some(updated) = item.updated_at else {
        return Ok(vec![original]);
    };
    if updated < item.created_at {
        return Err(NewsError::CorrectionPredatesOriginal {
            id: item.id,
            created: item.created_at,
            updated,
        });
    }
    if updated == item.created_at {
        // Alpaca sets updated_at == created_at on an untouched item.
        return Ok(vec![original]);
    }
    let revision = original.revise(
        item.headline.clone(),
        item.content.clone().or_else(|| item.summary.clone()),
        updated,
        ingested_time,
        declared_lag_ms,
    );
    Ok(vec![original, revision])
}

/// Parses a news page.
///
/// # Errors
/// A malformed payload.
pub fn parse_news(body: &str) -> Result<AlpacaNewsResponse, NewsError> {
    // Decoding a body fetched from an external REST API — see the note on the
    // `lint-no-json-hotpath` exception in `collectors::macro::fred`.
    serde_json::from_str::<AlpacaNewsResponse>(body).map_err(|e| NewsError::Parse(e.to_string()))
}

/// The budget every caller of this module must spend against.
#[must_use]
pub fn limiter() -> RateLimiter {
    RateLimiter::new(VenueLimit::alpaca_market_data())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dataplane::text::{item_as_of, validate_revision_chain};

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .expect("valid instant")
            .with_timezone(&Utc)
    }

    /// A stand-in for the `dataplane.source` row this feed will get. The tests
    /// only need the id to travel through unchanged; which number the migration
    /// picks is not this module's business.
    const TEST_SOURCE_ID: i32 = 20;

    const PAGE: &str = r#"{
      "news": [
        {"id": 1001, "headline": "Acme beats estimates", "summary": "s",
         "symbols": ["ACME"],
         "created_at": "2024-05-01T13:00:00Z", "updated_at": "2024-05-01T17:00:00Z"},
        {"id": 1002, "headline": "Other news", "symbols": ["BETA"],
         "created_at": "2024-05-01T14:00:00Z", "updated_at": "2024-05-01T14:00:00Z"}
      ],
      "next_page_token": "abc"
    }"#;

    #[test]
    fn a_page_parses() {
        let page = parse_news(PAGE).expect("fixture parses");
        assert_eq!(page.news.len(), 2);
        assert_eq!(page.next_page_token.as_deref(), Some("abc"));
    }

    /// A corrected item becomes two rows; an untouched one stays a single row.
    #[test]
    fn a_correction_becomes_a_second_row() {
        let page = parse_news(PAGE).expect("fixture parses");
        let corrected = to_text_items(&page.news[0], TEST_SOURCE_ID, t("2024-05-01T17:00:05Z"), 0)
            .expect("well-formed item");
        assert_eq!(corrected.len(), 2);
        assert_eq!(corrected[0].revision, 0);
        assert_eq!(corrected[1].revision, 1);
        assert_eq!(validate_revision_chain(&corrected), Ok(()));

        let untouched = to_text_items(&page.news[1], TEST_SOURCE_ID, t("2024-05-01T14:00:05Z"), 0)
            .expect("well-formed item");
        assert_eq!(untouched.len(), 1);
    }

    /// P-3.3 acceptance: a news item is never returned for a read before its
    /// publication time.
    #[test]
    fn an_item_is_never_returned_before_its_publication_time() {
        let page = parse_news(PAGE).expect("fixture parses");
        let rows = to_text_items(&page.news[0], TEST_SOURCE_ID, t("2024-05-01T17:00:05Z"), 0)
            .expect("well-formed item");
        assert!(item_as_of(&rows, "alpaca:1001", t("2024-05-01T12:59:59Z")).is_none());
        let at_15 = item_as_of(&rows, "alpaca:1001", t("2024-05-01T15:00:00Z"))
            .expect("published at 13:00");
        assert_eq!(at_15.revision, 0);
    }

    /// The knowledge time is the publication time, not the ingestion time. A
    /// collector that starts an hour late must not make an hour-old item look
    /// like it arrived when the collector woke up.
    #[test]
    fn ingestion_time_never_becomes_the_knowledge_time() {
        let page = parse_news(PAGE).expect("fixture parses");
        let rows = to_text_items(&page.news[1], TEST_SOURCE_ID, t("2024-05-01T20:00:00Z"), 0)
            .expect("well-formed item");
        assert_eq!(rows[0].available_time, t("2024-05-01T14:00:00Z"));
        assert_eq!(rows[0].ingested_time, t("2024-05-01T20:00:00Z"));
    }

    #[test]
    fn a_declared_lag_delays_availability() {
        let page = parse_news(PAGE).expect("fixture parses");
        let rows = to_text_items(
            &page.news[1],
            TEST_SOURCE_ID,
            t("2024-05-01T14:00:01Z"),
            900_000,
        )
        .expect("well-formed item");
        assert_eq!(rows[0].available_time, t("2024-05-01T14:15:00Z"));
    }

    /// Adversarial: a backwards correction is refused rather than stored. The
    /// week-old attempt's doc comment promised this and its code silently folded
    /// the item into a single row instead.
    #[test]
    fn a_correction_dated_before_its_original_is_refused() {
        let backwards = r#"{"news":[{"id":1003,"headline":"h","symbols":[],
          "created_at":"2024-05-01T13:00:00Z","updated_at":"2024-05-01T11:00:00Z"}]}"#;
        let page = parse_news(backwards).expect("fixture parses");
        assert!(matches!(
            to_text_items(&page.news[0], TEST_SOURCE_ID, t("2024-05-01T13:00:05Z"), 0),
            Err(NewsError::CorrectionPredatesOriginal { id: 1003, .. })
        ));
    }

    #[test]
    fn the_recorded_free_plan_depth_is_stated_here() {
        assert_eq!(FREE_PLAN_HISTORY_START, "2015-01-01");
    }
}
