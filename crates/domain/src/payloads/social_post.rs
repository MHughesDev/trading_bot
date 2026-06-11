//! `SocialPostPayload` — social-media post with instrument links.

use serde::{Deserialize, Serialize};

use crate::payloads::Payload;

/// A linked instrument mention found in a social-media post.
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
pub struct InstrumentMention {
    /// Domain instrument ID (e.g. `"BTC-USD"`).
    pub instrument_id: String,
    /// Confidence score in [0, 1].  Only mentions above a threshold are linked.
    pub confidence: f32,
}

/// A social-media post normalized for the platform.
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
pub struct SocialPostPayload {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rkyv_round_trip() {
        let p = SocialPostPayload::new(
            "reddit",
            "post-abc",
            "user123",
            "CryptoCurrency",
            "BTC to the moon",
            "I think $BTC will rally.",
            vec![InstrumentMention {
                instrument_id: "BTC-USD".to_owned(),
                confidence: 0.9,
            }],
            42,
        );
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&p).unwrap();
        // SAFETY: bytes were produced by rkyv::to_bytes immediately above.
        #[allow(unsafe_code)]
        let archived =
            unsafe { rkyv::access_unchecked::<rkyv::Archived<SocialPostPayload>>(bytes.as_ref()) };
        let back: SocialPostPayload =
            rkyv::deserialize::<_, rkyv::rancor::Error>(archived).unwrap();
        assert_eq!(p.score, back.score);
        assert_eq!(p.instrument_mentions.len(), back.instrument_mentions.len());
    }
}
