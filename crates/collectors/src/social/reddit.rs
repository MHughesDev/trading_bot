//! Reddit social collector — ingests posts + top-level comments via the official
//! OAuth Data API with two-stage instrument linking (P2-T11).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use domain::{
    payloads::social_post::{InstrumentMention, SocialPostPayload},
    EventEnvelope, NormalizeError,
};
use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::{Collector, CollectorError};

const REDDIT_OAUTH_BASE: &str = "https://oauth.reddit.com";
const VENUE_ID: &str = "reddit";
const SOURCE: &str = "reddit_oauth_api";

pub const LINK_CONFIDENCE_THRESHOLD: f32 = 0.75;

// ── Reddit API response shapes ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RedditListing {
    data: RedditListingData,
}

#[derive(Debug, Deserialize)]
struct RedditListingData {
    children: Vec<RedditChild>,
}

#[derive(Debug, Deserialize)]
struct RedditChild {
    data: RedditPost,
}

#[derive(Debug, Deserialize)]
struct RedditPost {
    id: String,
    author: Option<String>,
    subreddit: Option<String>,
    title: Option<String>,
    selftext: Option<String>,
    score: Option<i64>,
    body: Option<String>,
}

// ── Instrument linker ────────────────────────────────────────────────────────

pub fn extract_mentions(
    text: &str,
    known_instruments: &HashMap<String, String>,
    scratch: &mut HashMap<String, f32>,
) -> Vec<InstrumentMention> {
    scratch.clear();

    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '$' {
            let symbol: String = chars
                .by_ref()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            if symbol.len() >= 3 {
                let upper = symbol.to_uppercase();
                let confidence = if known_instruments.contains_key(&upper) {
                    1.0
                } else {
                    0.6
                };
                let entry = scratch.entry(upper).or_insert(0.0_f32);
                *entry = entry.max(confidence);
            }
        }
    }

    scratch
        .iter()
        .filter(|(_, conf)| **conf >= LINK_CONFIDENCE_THRESHOLD)
        .map(|(instrument_id, &confidence)| InstrumentMention {
            instrument_id: instrument_id.clone(),
            confidence,
        })
        .collect()
}

// ── Collector ────────────────────────────────────────────────────────────────

pub struct RedditCollector {
    pub subreddit: String,
    pub known_instruments: HashMap<String, String>,
}

impl RedditCollector {
    pub fn new(subreddit: impl Into<String>, known_instruments: HashMap<String, String>) -> Self {
        Self {
            subreddit: subreddit.into(),
            known_instruments,
        }
    }

    fn normalize_post(
        &self,
        post: &RedditPost,
        seq: u64,
        scratch: &mut HashMap<String, f32>,
    ) -> Result<EventEnvelope, NormalizeError> {
        let title = post.title.as_deref().unwrap_or_default();
        let body = post
            .body
            .as_deref()
            .or(post.selftext.as_deref())
            .unwrap_or_default();
        let combined = format!("{title} {body}");

        let mentions = extract_mentions(&combined, &self.known_instruments, scratch);
        debug!(
            post_id = %post.id,
            mention_count = mentions.len(),
            "instrument linking result"
        );

        let payload = SocialPostPayload::new(
            "reddit",
            &post.id,
            post.author.as_deref().unwrap_or("unknown"),
            post.subreddit.as_deref().unwrap_or(&self.subreddit),
            title,
            body,
            mentions,
            post.score.unwrap_or(0),
        );

        let payload_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&payload)
            .map_err(|e| NormalizeError::Deserialize(e.to_string()))?
            .into_vec();

        let instrument_name = format!("reddit.{}", self.subreddit);
        let timestamp_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0);

        Ok(EventEnvelope::new(
            domain::intern_instrument(&instrument_name),
            domain::intern_venue(VENUE_ID),
            domain::intern_source(SOURCE),
            seq,
            timestamp_ns,
            payload_bytes,
        ))
    }
}

#[async_trait]
impl Collector for RedditCollector {
    async fn run(
        &self,
        publisher: Arc<event_bus::Publisher>,
        quarantine: Arc<event_bus::QuarantinePublisher>,
    ) -> Result<(), CollectorError> {
        let client_id = std::env::var("REDDIT_CLIENT_ID").unwrap_or_default();
        let client_secret = std::env::var("REDDIT_CLIENT_SECRET").unwrap_or_default();
        let username = std::env::var("REDDIT_USERNAME").unwrap_or_default();
        let password = std::env::var("REDDIT_PASSWORD").unwrap_or_default();

        let client = reqwest::Client::new();
        let mut seq: u64 = 0;

        info!(subreddit = %self.subreddit, "Reddit collector starting");

        let token_resp = client
            .post("https://www.reddit.com/api/v1/access_token")
            .basic_auth(&client_id, Some(&client_secret))
            .form(&[
                ("grant_type", "password"),
                ("username", &username),
                ("password", &password),
            ])
            .header("User-Agent", "trading-bot/0.1")
            .send()
            .await
            .map_err(|e| CollectorError::Connect(e.to_string()))?;

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
        }

        let token: TokenResponse = token_resp
            .json()
            .await
            .map_err(|e| CollectorError::Connect(e.to_string()))?;

        let access_token = token.access_token;
        let mut mention_scratch: HashMap<String, f32> = HashMap::new();

        loop {
            let url = format!("{REDDIT_OAUTH_BASE}/r/{}/new?limit=25", self.subreddit);

            let result = client
                .get(&url)
                .header("Authorization", format!("bearer {access_token}"))
                .header("User-Agent", "trading-bot/0.1")
                .send()
                .await;

            match result {
                Err(e) => {
                    warn!(error = %e, subreddit = %self.subreddit, "Reddit API request failed");
                }
                Ok(resp) => {
                    let raw = resp.bytes().await.unwrap_or_default();
                    match serde_json::from_slice::<RedditListing>(&raw) {
                        Err(e) => {
                            let norm_err = NormalizeError::Deserialize(e.to_string());
                            if let Err(qe) =
                                quarantine.publish_failure(&raw, &norm_err, SOURCE).await
                            {
                                warn!(error = %qe, "quarantine publish failed");
                            }
                        }
                        Ok(listing) => {
                            let instrument_name = format!("reddit.{}", self.subreddit);
                            for child in &listing.data.children {
                                seq += 1;
                                let result =
                                    self.normalize_post(&child.data, seq, &mut mention_scratch);
                                crate::normalizer::quarantine_or_publish(
                                    result,
                                    &raw,
                                    &instrument_name,
                                    "social.post",
                                    SOURCE,
                                    &publisher,
                                    &quarantine,
                                )
                                .await;
                            }
                        }
                    }
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::payloads::social_post::SocialPostPayload;

    fn known() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("BTC".to_owned(), "BTC-USD".to_owned());
        m.insert("ETH".to_owned(), "ETH-USD".to_owned());
        m.insert("AAPL".to_owned(), "AAPL".to_owned());
        m
    }

    #[test]
    fn cashtag_btc_links_above_threshold() {
        let mut scratch = HashMap::new();
        let mentions = extract_mentions("$BTC is going to the moon!", &known(), &mut scratch);
        assert!(!mentions.is_empty(), "BTC cashtag should link");
        let btc = mentions.iter().find(|m| m.instrument_id == "BTC").unwrap();
        assert!(
            btc.confidence >= LINK_CONFIDENCE_THRESHOLD,
            "confidence below threshold: {}",
            btc.confidence
        );
    }

    #[test]
    fn ambiguous_two_char_ticker_does_not_link() {
        let mut scratch = HashMap::new();
        let mentions = extract_mentions("$AI is trending", &known(), &mut scratch);
        assert!(
            mentions.is_empty()
                || mentions
                    .iter()
                    .all(|m| m.confidence < LINK_CONFIDENCE_THRESHOLD),
            "short ticker should not link"
        );
    }

    #[test]
    fn unknown_cashtag_scores_below_threshold() {
        let mut scratch = HashMap::new();
        let mentions = extract_mentions("$DOGE is great", &known(), &mut scratch);
        assert!(
            mentions.is_empty(),
            "unknown cashtag should fall below threshold"
        );
    }

    #[test]
    fn multiple_cashtags_all_linked() {
        let mut scratch = HashMap::new();
        let mentions = extract_mentions("Buying $BTC and $ETH today", &known(), &mut scratch);
        assert_eq!(mentions.len(), 2, "both BTC and ETH should link");
    }

    #[test]
    fn normalize_post_builds_correct_payload() {
        let collector = RedditCollector::new("CryptoCurrency", known());
        let post = RedditPost {
            id: "abc123".to_owned(),
            author: Some("redditor".to_owned()),
            subreddit: Some("CryptoCurrency".to_owned()),
            title: Some("$BTC breaking out!".to_owned()),
            selftext: Some("I think $ETH follows.".to_owned()),
            score: Some(42),
            body: None,
        };
        let mut scratch = HashMap::new();
        let result = collector.normalize_post(&post, 1, &mut scratch);
        assert!(result.is_ok(), "{:?}", result);
        let env = result.unwrap();
        let payload: SocialPostPayload = env.decode_payload().unwrap();
        assert!(!payload.instrument_mentions.is_empty());
    }
}
