//! P2-T11 acceptance test: Reddit post linking with two-stage confidence filter.

use collectors::social::reddit::{extract_mentions, LINK_CONFIDENCE_THRESHOLD};
use std::collections::HashMap;

fn known() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("BTC".to_owned(), "BTC-USD".to_owned());
    m.insert("ETH".to_owned(), "ETH-USD".to_owned());
    m.insert("SPY".to_owned(), "SPY".to_owned());
    m
}

#[test]
fn cashtag_btc_links_above_threshold() {
    let mentions = extract_mentions("$BTC is looking bullish today!", &known());
    let btc = mentions.iter().find(|m| m.instrument_id == "BTC");
    assert!(btc.is_some(), "BTC cashtag should be linked");
    assert!(
        btc.unwrap().confidence >= LINK_CONFIDENCE_THRESHOLD,
        "confidence below threshold"
    );
}

#[test]
fn ambiguous_two_char_mention_falls_below_threshold() {
    let mentions = extract_mentions("$AI will replace us all", &known());
    // "AI" has 2 chars → extracted as 2-char ticker, filtered out.
    let linked = mentions
        .iter()
        .any(|m| m.instrument_id == "AI" && m.confidence >= LINK_CONFIDENCE_THRESHOLD);
    assert!(
        !linked,
        "short/ambiguous ticker should not link above threshold"
    );
}

#[test]
fn unknown_cashtag_does_not_link() {
    // DOGE not in known_instruments → confidence 0.6, below 0.75 threshold.
    let mentions = extract_mentions("$DOGE to the moon!", &known());
    assert!(
        mentions.is_empty(),
        "unknown cashtag should not link: {:?}",
        mentions
    );
}

#[test]
fn social_post_event_carries_confidence_score() {
    let mentions = extract_mentions("Buying $BTC and $ETH now", &known());
    assert_eq!(mentions.len(), 2);
    for m in &mentions {
        assert!(
            m.confidence >= LINK_CONFIDENCE_THRESHOLD,
            "linked mention must carry confidence >= threshold"
        );
    }
}
