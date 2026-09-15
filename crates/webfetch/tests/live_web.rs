//! Live-internet tests. `#[ignore]` by default, like every other suite here
//! that needs something outside the process.
//!
//! These exist because the parser in `search` is scraping a page nobody promised
//! us, and the only way to know it still works is to ask. When one of these
//! fails and the unit tests pass, the markup changed — that is the signal, and
//! it is why the two are separate suites.
//!
//! ```text
//! cargo test -p webfetch --test live_web -- --ignored --test-threads=1
//! ```

use std::sync::Arc;
use webfetch::egress::Policy;
use webfetch::fetch::{Fetcher, SystemResolver};
use webfetch::search::{Provider, SearchOutcome};

fn fetcher() -> Fetcher {
    Fetcher::new(Policy::default(), Arc::new(SystemResolver))
}

#[tokio::test]
#[ignore = "requires the internet"]
async fn a_real_page_fetches_and_extracts_to_readable_text() {
    let doc = fetcher()
        .get("https://en.wikipedia.org/wiki/Sharpe_ratio")
        .await
        .expect("fetch");

    assert_eq!(doc.status, 200);
    assert!(doc.content_type.contains("text/html"));
    let text = doc.text.read_as_untrusted_input();
    assert!(text.len() > 2_000, "extracted only {} chars", text.len());
    assert!(text.contains("Sharpe"), "the subject of the page should survive extraction");
    assert!(
        !text.contains("function(") && !text.contains("</script"),
        "script bodies must not reach the text"
    );
    assert_eq!(doc.body_sha256.len(), 64);
    assert!(doc.knowledge_time <= chrono::Utc::now());
}

/// The client's own canary, one test per provider. A failure here with green
/// unit tests means that provider's response shape moved.
#[tokio::test]
#[ignore = "requires the internet"]
async fn every_provider_answers_with_parseable_results() {
    let f = fetcher();
    let mut worked = 0;
    for provider in Provider::ALL {
        // One query per provider, matched to what it indexes. Asking an
        // encyclopedia a four-term paper query and calling the empty result a
        // failure would be a bug in the test, not the client.
        let query = match provider {
            Provider::Wikipedia => "Sharpe ratio",
            _ => "deflated sharpe ratio backtest overfitting",
        };
        match f.search(*provider, query, 5).await {
            Ok(SearchOutcome::Found { hits }) => {
                worked += 1;
                assert!(!hits.is_empty());
                for h in &hits {
                    assert!(h.url.starts_with("http"), "{provider:?}: {}", h.url);
                    assert!(!h.title.read_as_untrusted_input().is_empty(), "{provider:?}");
                }
            }
            Ok(SearchOutcome::Empty) => {
                panic!("{provider:?} returned nothing for {query:?}, which has a known literature")
            }
            Ok(SearchOutcome::Unrecognised { reason, bytes }) => {
                // arXiv rate-limits hard from shared addresses; that is a fact
                // about the service, not a broken parser, so it does not fail
                // the suite on its own.
                assert_eq!(
                    *provider,
                    Provider::Arxiv,
                    "{provider:?} response shape changed ({bytes} bytes): {reason}"
                );
            }
            Err(e) => assert_eq!(*provider, Provider::Arxiv, "{provider:?} failed: {e}"),
        }
    }
    assert!(worked >= 2, "at least two providers must be working");
}

/// The field the novelty check exists for. A scraped results page does not
/// carry it and these do.
#[tokio::test]
#[ignore = "requires the internet"]
async fn scholarly_results_carry_publication_years() {
    let out = fetcher()
        .search(Provider::OpenAlex, "deflated sharpe ratio", 10)
        .await
        .expect("search");
    let SearchOutcome::Found { hits } = out else { panic!("expected hits") };
    let dated = hits.iter().filter(|h| h.published_year.is_some()).count();
    assert!(dated >= hits.len() / 2, "only {dated}/{} hits carried a year", hits.len());
    assert!(
        hits.iter().any(|h| h.cited_by.is_some()),
        "citation counts are how crowding is measured"
    );
}

/// The allowlist is not decoration.
#[tokio::test]
#[ignore = "requires the internet"]
async fn an_unlisted_host_is_refused_without_a_packet_leaving() {
    let err = fetcher()
        .get("https://example.com/")
        .await
        .expect_err("must refuse");
    assert!(err.to_string().contains("egress allowlist"), "{err}");
}

/// Belt and braces against the worst case: even with the host allowed, the
/// address rules refuse the metadata endpoint.
#[tokio::test]
#[ignore = "requires the internet"]
async fn the_metadata_endpoint_is_refused_even_when_named_directly() {
    let open = Policy {
        hosts: webfetch::egress::HostPolicy::AllowAny,
        ..Policy::default()
    };
    let err = Fetcher::new(open, Arc::new(SystemResolver))
        .get("https://169.254.169.254/latest/meta-data/")
        .await
        .expect_err("must refuse");
    assert!(err.to_string().contains("metadata"), "{err}");
}

/// A binary response is refused by name rather than decoded into a page of
/// replacement characters.
#[tokio::test]
#[ignore = "requires the internet"]
async fn a_pdf_is_refused_rather_than_mangled() {
    let result = fetcher().get("https://arxiv.org/pdf/1512.03385").await;
    match result {
        Err(webfetch::FetchError::UnsupportedContentType { content_type, .. }) => {
            assert!(content_type.contains("pdf"), "{content_type}");
        }
        Err(other) => panic!("wrong error: {other}"),
        Ok(doc) => panic!("a PDF should not have parsed as text ({} bytes)", doc.bytes),
    }
}
