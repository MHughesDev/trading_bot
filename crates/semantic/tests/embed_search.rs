//! P7-T03 acceptance tests — Milvus embedding and metadata-filtered search.
//!
//! Live tests require a running Milvus instance and OPENAI_API_KEY.

use semantic::embed::EmbedSource;

/// EmbedSource produces correct source strings.
#[test]
fn embed_source_produces_correct_strings() {
    assert_eq!(EmbedSource::SocialPost.as_str(), "social.post");
    assert_eq!(EmbedSource::WebPageSnapshot.as_str(), "web.page_snapshot");
    assert_eq!(
        EmbedSource::StrategyDescription.as_str(),
        "strategy.description"
    );
}

/// Collection field constants are non-empty strings.
#[test]
fn collection_field_constants_are_non_empty() {
    assert!(!semantic::collection::fields::VECTOR.is_empty());
    assert!(!semantic::collection::fields::SOURCE.is_empty());
    assert!(!semantic::collection::fields::EVENT_ID.is_empty());
    assert!(!semantic::collection::fields::TEXT.is_empty());
}

/// Source constants match EmbedSource string values.
#[test]
fn source_constants_match_embed_source() {
    assert_eq!(
        semantic::collection::sources::SOCIAL_POST,
        EmbedSource::SocialPost.as_str()
    );
    assert_eq!(
        semantic::collection::sources::WEB_PAGE_SNAPSHOT,
        EmbedSource::WebPageSnapshot.as_str()
    );
    assert_eq!(
        semantic::collection::sources::STRATEGY_DESCRIPTION,
        EmbedSource::StrategyDescription.as_str()
    );
}

/// Live: embed three texts (one per source), search filtered to social.post,
/// result excludes web and strategy sources.
#[tokio::test]
#[ignore = "requires Milvus + OPENAI_API_KEY env var"]
async fn filtered_semantic_search_returns_correct_source() {
    use semantic::embed::EmbeddingRequest;
    use semantic::{CollectionSpec, MilvusClient, MilvusConfig};

    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set");
    let cfg = MilvusConfig::from_env();
    let client = MilvusClient::connect(cfg, CollectionSpec::social_posts())
        .await
        .expect("connect failed");

    client
        .ensure_collection()
        .await
        .expect("ensure_collection failed");

    let samples = [
        (
            EmbedSource::SocialPost,
            "BTC is going to break 100k soon",
            "reddit",
        ),
        (
            EmbedSource::WebPageSnapshot,
            "Fed raises interest rates by 25bps",
            "web",
        ),
        (
            EmbedSource::StrategyDescription,
            "EMA crossover strategy on crypto assets",
            "platform",
        ),
    ];

    for (i, (source, text, venue)) in samples.iter().enumerate() {
        let req = EmbeddingRequest {
            text: text.to_string(),
            source: source.clone(),
            event_id: format!("test-{i}"),
            instrument_ids: "BTC-USDT".into(),
            venue: venue.to_string(),
            occurred_at_ms: 0,
        };
        client
            .embed_and_upsert(&req, &api_key)
            .await
            .expect("embed_and_upsert failed");
    }

    // Search filtered to social.post — web and strategy sources must not appear.
    let results = client
        .search_similar("Bitcoin price prediction", Some("social.post"), 5, &api_key)
        .await
        .expect("search failed");

    assert!(!results.is_empty(), "should return at least one result");
    for r in &results {
        assert_eq!(
            r.source, "social.post",
            "filtered search returned wrong source: {}",
            r.source
        );
    }
}
