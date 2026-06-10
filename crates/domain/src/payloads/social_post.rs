//! `SocialPostPayload` — social-media post with instrument links.

use serde::{Deserialize, Serialize};

use crate::payloads::Payload;

/// A linked instrument mention found in a social-media post.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstrumentMention {
    /// Domain instrument ID (e.g. `"BTC-USD"`).
    pub instrument_id: String,
    /// Confidence score in [0, 1].  Only mentions above a threshold are linked.
    pub confidence: f32,
}

/// A social-media post normalized for the platform.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SocialPostPayload {
    pub schema_version: String,
    /// Source platform (e.g. `"reddit"`).
    pub source_platform: String,
    /// Opaque platform-assigned post ID.
    pub post_id: String,
    /// Author handle (scrubbed of PII as needed).
    pub author: String,
    /// Subreddit or channel name.
    pub channel: String,
    /// Post title (or empty for comment-only posts).
    pub title: String,
    /// Post or comment body text.
    pub body: String,
    /// Instrument mentions passing the confidence threshold.
    pub instrument_mentions: Vec<InstrumentMention>,
    /// Upvote count at time of ingestion.
    pub score: i64,
}

impl SocialPostPayload {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source_platform: impl Into<String>,
        post_id: impl Into<String>,
        author: impl Into<String>,
        channel: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
        instrument_mentions: Vec<InstrumentMention>,
        score: i64,
    ) -> Self {
        Self {
            schema_version: Self::schema_version().into(),
            source_platform: source_platform.into(),
            post_id: post_id.into(),
            author: author.into(),
            channel: channel.into(),
            title: title.into(),
            body: body.into(),
            instrument_mentions,
            score,
        }
    }
}

impl Payload for SocialPostPayload {
    fn event_type() -> &'static str {
        "social.post.v1"
    }

    fn schema_version() -> &'static str {
        "1"
    }
}
