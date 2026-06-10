//! P7-T04 acceptance tests — web scraper compliance.
//!
//! Verifies robots.txt enforcement, per-domain rate limiting, Playwright
//! fallback triggering, and web.page_snapshot event structure.
//! All tests run without live network access.

use collectors::web::scraper::{
    extract_domain, extract_title, needs_playwright_fallback, strip_html, PerDomainRateLimiter,
    RobotsTxt, WebScraperConfig,
};
use std::time::Duration;

// ── robots.txt compliance ─────────────────────────────────────────────────────

#[test]
fn robots_disallowed_path_is_skipped() {
    let r = RobotsTxt::parse("User-agent: *\nDisallow: /private/\n");
    assert!(
        !r.is_allowed("/private/secret"),
        "disallowed path must be rejected"
    );
}

#[test]
fn robots_allowed_path_passes() {
    let r = RobotsTxt::parse("User-agent: *\nDisallow: /private/\n");
    assert!(r.is_allowed("/public/page"), "allowed path must pass");
}

#[test]
fn robots_empty_allows_all() {
    let r = RobotsTxt::parse("");
    assert!(r.is_allowed("/any/path"));
}

#[test]
fn robots_respects_trading_bot_agent() {
    let content = "User-agent: trading-bot\nDisallow: /restricted/\n\nUser-agent: *\nDisallow:\n";
    let r = RobotsTxt::parse(content);
    assert!(
        !r.is_allowed("/restricted/data"),
        "trading-bot disallow must apply"
    );
    assert!(r.is_allowed("/open/data"), "open paths must be allowed");
}

#[test]
fn robots_root_disallow_blocks_all() {
    let r = RobotsTxt::parse("User-agent: *\nDisallow: /\n");
    assert!(!r.is_allowed("/"), "/ must be disallowed");
    assert!(!r.is_allowed("/any/page"), "all paths must be disallowed");
}

// ── Per-domain rate limiting ─────────────────────────────────────────────────

#[test]
fn rate_limiter_allows_first_request() {
    let l = PerDomainRateLimiter::new(Duration::from_secs(30));
    assert!(l.is_ready("news.example.com"));
}

#[test]
fn rate_limiter_throttles_after_record() {
    let mut l = PerDomainRateLimiter::new(Duration::from_secs(30));
    l.record("news.example.com");
    assert!(
        !l.is_ready("news.example.com"),
        "domain must be throttled immediately after a request"
    );
}

#[test]
fn rate_limiter_does_not_throttle_different_domain() {
    let mut l = PerDomainRateLimiter::new(Duration::from_secs(30));
    l.record("a.com");
    assert!(
        l.is_ready("b.com"),
        "different domain must not be throttled"
    );
}

// ── Playwright fallback detection ────────────────────────────────────────────

#[test]
fn playwright_fallback_triggered_on_empty_content() {
    assert!(needs_playwright_fallback(""));
    assert!(needs_playwright_fallback("  "));
    assert!(needs_playwright_fallback("short"));
}

#[test]
fn playwright_fallback_not_triggered_on_rich_content() {
    let rich =
        "Bitcoin hits new all-time high as institutional buyers flood the market. ".repeat(5);
    assert!(!needs_playwright_fallback(&rich));
}

// ── web.page_snapshot event ──────────────────────────────────────────────────

#[test]
fn web_page_snapshot_payload_constructs() {
    use domain::payloads::web_page_snapshot::{FetchMethod, WebPageSnapshotPayload};

    let payload = WebPageSnapshotPayload::new(
        "https://example.com/article",
        "example.com",
        "Example Article",
        "This is the article body text with some useful content.",
        200_u16,
        FetchMethod::Http,
        2048_usize,
    );

    assert_eq!(payload.status_code, 200);
    assert_eq!(payload.fetch_method, FetchMethod::Http);
    assert!(!payload.text_content.is_empty());
    assert_eq!(payload.domain, "example.com");
}

#[test]
fn playwright_fallback_fetch_method_differs_from_http() {
    use domain::payloads::web_page_snapshot::FetchMethod;
    assert_ne!(FetchMethod::Http, FetchMethod::Playwright);
}

// ── Content helpers ───────────────────────────────────────────────────────────

#[test]
fn strip_html_removes_tags() {
    assert_eq!(strip_html("<h1>Hello</h1><p>World</p>"), "Hello World");
}

#[test]
fn extract_title_finds_title_tag() {
    let html = "<html><head><title>Breaking News</title></head><body>...</body></html>";
    assert_eq!(extract_title(html), "Breaking News");
}

#[test]
fn extract_domain_works() {
    assert_eq!(
        extract_domain("https://finance.example.com/news/1"),
        Some("finance.example.com".into())
    );
}

// ── WebScraperConfig defaults ─────────────────────────────────────────────────

#[test]
fn web_scraper_config_default() {
    let cfg = WebScraperConfig::default();
    assert!(cfg.urls.is_empty());
    assert!(cfg.rate_limit > Duration::ZERO);
    assert!(cfg.poll_interval > Duration::ZERO);
}
