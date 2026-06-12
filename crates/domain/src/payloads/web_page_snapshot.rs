//! `WebPageSnapshotPayload` — scraped web-page snapshot.

use serde::{Deserialize, Serialize};

use crate::payloads::Payload;

/// How the page was fetched.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[rkyv(derive(Debug, PartialEq))]
#[serde(rename_all = "snake_case")]
pub enum FetchMethod {
    Http,
    Playwright,
}

/// A scraped web-page snapshot normalized for the platform.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[rkyv(derive(Debug))]
pub struct WebPageSnapshotPayload {
    /// Full URL of the fetched page.
    pub url: String,
    /// Registered domain (e.g. `"example.com"`).
    pub domain: String,
    /// Page title extracted from `<title>` tag.
    pub title: String,
    /// Visible text content (HTML tags stripped).
    pub text_content: String,
    /// HTTP status code returned by the server.
    pub status_code: u16,
    /// Whether the page was fetched via HTTP or Playwright fallback.
    pub fetch_method: FetchMethod,
    /// Byte length of the original HTML response.
    pub content_length: u64,
}

impl WebPageSnapshotPayload {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        url: impl Into<String>,
        domain: impl Into<String>,
        title: impl Into<String>,
        text_content: impl Into<String>,
        status_code: u16,
        fetch_method: FetchMethod,
        content_length: usize,
    ) -> Self {
        Self {
            url: url.into(),
            domain: domain.into(),
            title: title.into(),
            text_content: text_content.into(),
            status_code,
            fetch_method,
            content_length: content_length as u64,
        }
    }
}

impl Payload for WebPageSnapshotPayload {
    fn event_type() -> &'static str {
        // Must match the NATS lane used in the web scraper (WEB_PAGE_SNAPSHOT_LANE).
        // Previously returned "web.page_snapshot.v1" which created a mismatch
        // between the event_type field and the lane consumers subscribe to (M-10).
        "web.page_snapshot"
    }

    fn schema_version() -> &'static str {
        "1"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rkyv_round_trip() {
        let p = WebPageSnapshotPayload::new(
            "https://example.com/btc",
            "example.com",
            "BTC News",
            "Bitcoin is rising today.",
            200,
            FetchMethod::Http,
            1024,
        );
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&p).unwrap();
        // SAFETY: bytes were produced by rkyv::to_bytes immediately above.
        #[allow(unsafe_code)]
        let archived = unsafe {
            rkyv::access_unchecked::<rkyv::Archived<WebPageSnapshotPayload>>(bytes.as_ref())
        };
        let back: WebPageSnapshotPayload =
            rkyv::deserialize::<_, rkyv::rancor::Error>(archived).unwrap();
        assert_eq!(p.status_code, back.status_code);
        assert_eq!(p.content_length, back.content_length);
        assert_eq!(p.fetch_method, back.fetch_method);
    }
}
