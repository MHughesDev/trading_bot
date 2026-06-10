//! `WebPageSnapshotPayload` — scraped web-page snapshot.

use serde::{Deserialize, Serialize};

use crate::payloads::Payload;

/// How the page was fetched.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchMethod {
    Http,
    Playwright,
}

/// A scraped web-page snapshot normalized for the platform.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebPageSnapshotPayload {
    pub schema_version: String,
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
    pub content_length: usize,
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
            schema_version: Self::schema_version().into(),
            url: url.into(),
            domain: domain.into(),
            title: title.into(),
            text_content: text_content.into(),
            status_code,
            fetch_method,
            content_length,
        }
    }
}

impl Payload for WebPageSnapshotPayload {
    fn event_type() -> &'static str {
        "web.page_snapshot.v1"
    }

    fn schema_version() -> &'static str {
        "1"
    }
}
