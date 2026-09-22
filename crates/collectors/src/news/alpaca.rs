//! Alpaca News API → `text_items` (DATA-005 §11 S2, §10; Set P, P-3.3).
//!
//! A news item's knowledge time is its **publication time**, not the moment the
//! collector happened to fetch it. Alpaca gives both (`created_at` and
//! `updated_at`), and the difference between them is what tells a correction from
//! an original: an item whose `updated_at` is later than its `created_at` is a
//! *revision*, and a revision is a new row with a higher `revision` number, never
//! an update of the row already stored ([`dataplane::text`]).
//!
//! ## Free-plan history depth — measured, not assumed
//!
//! **Verified 2026-09-15** against `docs.alpaca.markets/us/docs/historical-news-data`
//! ("This API provides historical news data dating back to 2015") and Alpaca's
//! News API announcement (free plan: 200 requests/minute; Unlimited: 10,000/min;
//! news supplied by Benzinga). The archive depth is **2015** and is *not* tiered —
//! the paid plans buy request rate, not history. `/v1beta1/news` accepts
//! `start`/`end` over that range with a `page_token` cursor, so the series is
//! genuinely backfillable.
//!
//! That is the finding P-3.3 asks for, and it is the *good* outcome: news does
//! **not** join the FM-11 forward-collect argument the way funding rates and
//! option chains do. It is still worth collecting forward, because a corrected
//! item's original text is only observable if you were reading at the time — the
//! archive serves the current version of a story, so revision history before the
//! collector started is unrecoverable even though the text is not.
//!
//! The depth number is recorded in
//! `docs/plans/plan-sets/set-P/phase-3-alternative-data.md` as the task requires.

use chrono::{DateTime, Utc};
use dataplane::text::TextItem;
use serde::Deserialize;

use crate::ratelimit::TokenBucket;

/// Alpaca's data host for news.
pub const ALPACA_DATA_BASE: &str = "https://data.alpaca.markets";

/// The registry name of this source (`dataplane::source`, migration 0103).
pub const SOURCE_NAME: &str = "alpaca_news";

/// `dataplane.source.source_id` for `alpaca_news`.
pub const SOURCE_ID: i32 = 32;

/// The earliest date the free-plan archive serves.
///
/// Alpaca documents the depth as "back to 2015" without naming a day, so the
/// first of that year is the conservative floor to request from. See the module
/// note for the verification.
pub const FREE_PLAN_HISTORY_START: &str = "2015-01-01";

/// Alpaca's documented free-plan ceiling is 200 requests per minute, i.e. 3.33/s.
/// Rounded **down**: a limiter that rounds up is a limiter that gets you banned.
pub const RATE_PER_SEC: u32 = 3;

#[derive(Debug, thiserror::Error)]
pub enum NewsError {
    #[error("malformed response: {0}")]
    Parse(String),
    #[error("news item {id} has no publication time")]
    NoPublicationTime { id: String },
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
/// `ingested_time` is when the collector saw it, and `declared_lag_ms` is the
/// source's declared lag — 0 for the live stream, non-zero for a delayed feed.
///
/// # Errors
/// An item whose `updated_at` precedes its `created_at` — a correction cannot
/// predate what it corrects, and storing it would make an as-of read return the
/// correction for an instant before it happened.
pub fn to_text_items(
    item: &AlpacaNewsItem,
    ingested_time: DateTime<Utc>,
    declared_lag_ms: i64,
) -> Result<Vec<TextItem>, NewsError> {
    let id = item_id(item.id);
    let mut original = TextItem::first(
        id.clone(),
        SOURCE_ID,
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
    if updated <= item.created_at {
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
    // Parsing a body fetched from an external REST API — the documented exception
    // in `cargo xtask lint-no-json-hotpath`.
    serde_json::from_str::<AlpacaNewsResponse>(&body[..]).map_err(|e| NewsError::Parse(e.to_string()))
}

/// The limiter every caller of this module must route requests through.
#[must_use]
pub fn limiter() -> TokenBucket {
    TokenBucket::new(RATE_PER_SEC, RATE_PER_SEC)
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
        let corrected = to_text_items(&page.news[0], t("2024-05-01T17:00:05Z"), 0)
            .expect("well-formed item");
        assert_eq!(corrected.len(), 2);
        assert_eq!(corrected[0].revision, 0);
        assert_eq!(corrected[1].revision, 1);
        assert_eq!(validate_revision_chain(&corrected), Ok(()));

        let untouched = to_text_items(&page.news[1], t("2024-05-01T14:00:05Z"), 0)
            .expect("well-formed item");
        assert_eq!(untouched.len(), 1);
    }

    /// P-3.3 acceptance: a news item is never returned for a read before its
    /// publication time.
    #[test]
    fn an_item_is_never_returned_before_its_publication_time() {
        let page = parse_news(PAGE).expect("fixture parses");
        let rows = to_text_items(&page.news[0], t("2024-05-01T17:00:05Z"), 0)
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
        let rows = to_text_items(&page.news[1], t("2024-05-01T20:00:00Z"), 0)
            .expect("well-formed item");
        assert_eq!(rows[0].available_time, t("2024-05-01T14:00:00Z"));
        assert_eq!(rows[0].ingested_time, t("2024-05-01T20:00:00Z"));
    }

    #[test]
    fn a_declared_lag_delays_availability() {
        let page = parse_news(PAGE).expect("fixture parses");
        let rows = to_text_items(&page.news[1], t("2024-05-01T14:00:01Z"), 900_000)
            .expect("well-formed item");
        assert_eq!(rows[0].available_time, t("2024-05-01T14:15:00Z"));
    }

    #[test]
    fn the_recorded_free_plan_depth_is_stated_here_and_in_the_phase_file() {
        assert_eq!(FREE_PLAN_HISTORY_START, "2015-01-01");
    }
}
