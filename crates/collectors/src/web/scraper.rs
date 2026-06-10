//! Web page scraper — HTTP-first, robots.txt-compliant, rate-limited.
//!
//! Fetches web pages and emits `EventEnvelope<WebPageSnapshotPayload>`.
//! `robots.txt` is honoured for all paths and per-domain rate limits are
//! enforced.  When HTTP yields insufficient text content, a Playwright
//! subprocess is invoked as a fallback if `PLAYWRIGHT_BIN` is configured.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use domain::{
    event_id_from_key,
    payloads::web_page_snapshot::{FetchMethod, WebPageSnapshotPayload},
    sequenced_key, EventEnvelope, TrustTier,
};
use tracing::{debug, info, warn};

use crate::{Collector, CollectorError};

const VENUE_ID: &str = "web";
const SOURCE: &str = "web_scraper";
/// Minimum word count to treat an HTTP response as having usable content.
pub const MIN_WORD_COUNT: usize = 20;
/// Cache TTL for robots.txt entries.
const ROBOTS_TTL: Duration = Duration::from_secs(3600);

// ── robots.txt ────────────────────────────────────────────────────────────────

/// A parsed `robots.txt` for a single domain.
#[derive(Debug, Clone)]
pub struct RobotsTxt {
    /// Disallowed path prefixes for our user-agent (`*` or `trading-bot`).
    disallowed: Vec<String>,
    fetched_at: std::time::SystemTime,
}

impl RobotsTxt {
    /// Parse raw robots.txt content, extracting `Disallow` rules for `*`
    /// and `trading-bot` user-agents.
    pub fn parse(content: &str) -> Self {
        let mut disallowed: Vec<String> = Vec::new();
        let mut in_applicable_section = false;

        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("User-agent:") {
                let ua = rest.trim().to_lowercase();
                in_applicable_section = ua == "*" || ua == "trading-bot";
            } else if in_applicable_section {
                if let Some(rest) = line.strip_prefix("Disallow:") {
                    let path = rest.trim().to_owned();
                    if !path.is_empty() {
                        disallowed.push(path);
                    }
                }
            }
        }

        Self {
            disallowed,
            fetched_at: std::time::SystemTime::now(),
        }
    }

    /// Return `true` if `path` is allowed for our user-agent.
    pub fn is_allowed(&self, path: &str) -> bool {
        !self
            .disallowed
            .iter()
            .any(|disallow| path.starts_with(disallow.as_str()))
    }

    pub(crate) fn is_stale(&self) -> bool {
        self.fetched_at
            .elapsed()
            .map(|e| e > ROBOTS_TTL)
            .unwrap_or(true)
    }
}

// ── Per-domain rate limiter ───────────────────────────────────────────────────

/// Simple per-domain last-access rate limiter.
#[derive(Debug)]
pub struct PerDomainRateLimiter {
    /// Minimum interval between successive requests to the same domain.
    min_interval: Duration,
    last_seen: HashMap<String, Instant>,
}

impl PerDomainRateLimiter {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_seen: HashMap::new(),
        }
    }

    /// Return `true` when a request to `domain` is permitted right now.
    pub fn is_ready(&self, domain: &str) -> bool {
        self.last_seen
            .get(domain)
            .map(|t| t.elapsed() >= self.min_interval)
            .unwrap_or(true)
    }

    /// Record that a request to `domain` was just made.
    pub fn record(&mut self, domain: &str) {
        self.last_seen.insert(domain.to_owned(), Instant::now());
    }
}

// ── Content helpers ───────────────────────────────────────────────────────────

/// Strip HTML tags and collapse whitespace to return visible text content.
pub fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extract the page title from a `<title>...</title>` element.
pub fn extract_title(html: &str) -> String {
    let start = html
        .find("<title")
        .and_then(|i| html[i..].find('>').map(|j| i + j + 1));
    let end = html.find("</title>");
    match (start, end) {
        (Some(s), Some(e)) if s < e => html[s..e].trim().to_owned(),
        _ => String::new(),
    }
}

/// Return `true` if the text content is below the minimum word threshold,
/// indicating that Playwright fallback should be attempted.
pub fn needs_playwright_fallback(text_content: &str) -> bool {
    text_content.split_whitespace().count() < MIN_WORD_COUNT
}

/// Extract the domain (host) from an HTTP or HTTPS URL.
pub fn extract_domain(url: &str) -> Option<String> {
    let rest = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    rest.split('/').next().map(|h| h.to_lowercase())
}

// ── Playwright fallback ───────────────────────────────────────────────────────

/// Attempt a Playwright fetch if `PLAYWRIGHT_BIN` is configured.
///
/// Spawns the Playwright CLI to render the page and extract `innerText`.
/// Returns `None` when `PLAYWRIGHT_BIN` is not set or the subprocess fails.
fn try_playwright_fetch(url: &str) -> Option<String> {
    let bin = std::env::var("PLAYWRIGHT_BIN").ok()?;
    let script = format!(
        "(async () => {{ \
         const {{ chromium }} = require('playwright'); \
         const b = await chromium.launch(); \
         const p = await b.newPage(); \
         await p.goto('{url}', {{ timeout: 15000 }}); \
         const t = await p.innerText('body').catch(() => ''); \
         await b.close(); \
         process.stdout.write(t); \
         }})()"
    );
    let output = std::process::Command::new(&bin)
        .args(["eval", &script])
        .output()
        .ok()?;

    if output.status.success() {
        String::from_utf8(output.stdout).ok()
    } else {
        warn!(%url, "Playwright fetch failed");
        None
    }
}

// ── Web scraper ───────────────────────────────────────────────────────────────

/// Configuration for the web scraper satellite.
#[derive(Debug, Clone)]
pub struct WebScraperConfig {
    /// URLs to scrape on each pass.
    pub urls: Vec<String>,
    /// Minimum interval between requests to the same domain.
    pub rate_limit: Duration,
    /// Interval between full scrape passes.
    pub poll_interval: Duration,
}

impl Default for WebScraperConfig {
    fn default() -> Self {
        Self {
            urls: vec![],
            rate_limit: Duration::from_secs(2),
            poll_interval: Duration::from_secs(300),
        }
    }
}

/// Web page scraper satellite.
pub struct WebScraper {
    config: WebScraperConfig,
    http: reqwest::Client,
}

impl WebScraper {
    pub fn new(config: WebScraperConfig) -> Self {
        let http = reqwest::Client::builder()
            .user_agent("trading-bot/0.1 (+https://github.com/trading-bot)")
            .timeout(Duration::from_secs(15))
            .build()
            .expect("failed to build HTTP client");
        Self { config, http }
    }

    async fn fetch_robots(&self, domain: &str) -> RobotsTxt {
        let url = format!("https://{domain}/robots.txt");
        match self.http.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let content = resp.text().await.unwrap_or_default();
                debug!(%domain, "fetched robots.txt");
                RobotsTxt::parse(&content)
            }
            _ => {
                debug!(%domain, "robots.txt unavailable — allow all");
                RobotsTxt::parse("")
            }
        }
    }

    async fn fetch_page(
        &self,
        url: &str,
        seq: u64,
    ) -> Option<EventEnvelope<WebPageSnapshotPayload>> {
        let domain = extract_domain(url)?;
        let path = url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches(&*domain)
            .to_owned();
        let path = if path.is_empty() {
            "/".to_owned()
        } else {
            path
        };

        // Check robots.txt before fetching.
        let robots = self.fetch_robots(&domain).await;
        if !robots.is_allowed(&path) {
            info!(%url, "robots.txt disallows crawl — skipping");
            return None;
        }

        let http_resp = self.http.get(url).send().await.ok()?;
        let status = http_resp.status().as_u16();
        let html = http_resp.text().await.unwrap_or_default();
        let content_length = html.len();

        let title = extract_title(&html);
        let mut text_content = strip_html(&html);
        let mut fetch_method = FetchMethod::Http;

        // Playwright fallback when HTTP yields insufficient content.
        if needs_playwright_fallback(&text_content) {
            if let Some(pw_content) = try_playwright_fetch(url) {
                text_content = pw_content;
                fetch_method = FetchMethod::Playwright;
                debug!(%url, "used Playwright fallback");
            }
        }

        let payload = WebPageSnapshotPayload::new(
            url,
            &domain,
            title,
            text_content,
            status,
            fetch_method,
            content_length,
        );

        let dedup = sequenced_key("web.page_snapshot", url, VENUE_ID, seq, SOURCE);
        let event_id = event_id_from_key(&dedup);
        let now = Utc::now();

        Some(EventEnvelope::new(
            event_id,
            "web.page_snapshot",
            domain,
            VENUE_ID,
            SOURCE,
            TrustTier::OnchainTentative,
            None,
            now,
            now,
            now,
            seq,
            payload,
        ))
    }
}

#[async_trait]
impl Collector for WebScraper {
    async fn run(
        &self,
        publisher: Arc<event_bus::Publisher>,
        quarantine: Arc<event_bus::QuarantinePublisher>,
    ) -> Result<(), CollectorError> {
        info!(url_count = self.config.urls.len(), "web scraper starting");

        let mut rate_limiter = PerDomainRateLimiter::new(self.config.rate_limit);
        let mut robots_cache: HashMap<String, RobotsTxt> = HashMap::new();
        let mut seq: u64 = 0;

        loop {
            for url in &self.config.urls {
                let domain = match extract_domain(url) {
                    Some(d) => d,
                    None => {
                        warn!(%url, "could not extract domain — skipping");
                        continue;
                    }
                };

                // Refresh stale robots.txt entries.
                if robots_cache
                    .get(&domain)
                    .map(RobotsTxt::is_stale)
                    .unwrap_or(true)
                {
                    robots_cache.insert(domain.clone(), self.fetch_robots(&domain).await);
                }

                let path = url
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .trim_start_matches(&*domain)
                    .to_owned();
                let path = if path.is_empty() {
                    "/".to_owned()
                } else {
                    path
                };

                if !robots_cache
                    .get(&domain)
                    .map(|r| r.is_allowed(&path))
                    .unwrap_or(true)
                {
                    debug!(%url, "robots.txt disallows — skipping");
                    continue;
                }

                if !rate_limiter.is_ready(&domain) {
                    debug!(%domain, "rate limited — skipping this pass");
                    continue;
                }
                rate_limiter.record(&domain);

                seq += 1;
                if let Some(envelope) = self.fetch_page(url, seq).await {
                    let raw = serde_json::to_vec(&envelope).unwrap_or_default();
                    crate::normalizer::quarantine_or_publish(
                        Ok(envelope),
                        &raw,
                        "web.page_snapshot",
                        SOURCE,
                        &publisher,
                        &quarantine,
                    )
                    .await;
                }
            }

            tokio::time::sleep(self.config.poll_interval).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robots_txt_disallowed_path_is_rejected() {
        let r = RobotsTxt::parse("User-agent: *\nDisallow: /private/\nDisallow: /admin/\n");
        assert!(
            !r.is_allowed("/private/data"),
            "/private/ should be disallowed"
        );
        assert!(
            !r.is_allowed("/admin/panel"),
            "/admin/ should be disallowed"
        );
        assert!(r.is_allowed("/public/page"), "/public/ should be allowed");
    }

    #[test]
    fn robots_txt_empty_allows_all() {
        let r = RobotsTxt::parse("");
        assert!(r.is_allowed("/anything"));
    }

    #[test]
    fn robots_txt_root_disallow_blocks_all() {
        let r = RobotsTxt::parse("User-agent: *\nDisallow: /\n");
        assert!(!r.is_allowed("/"), "/ should be disallowed");
        assert!(!r.is_allowed("/page"), "all paths should be disallowed");
    }

    #[test]
    fn robots_txt_trading_bot_agent() {
        let content =
            "User-agent: trading-bot\nDisallow: /restricted/\n\nUser-agent: *\nDisallow:\n";
        let r = RobotsTxt::parse(content);
        assert!(!r.is_allowed("/restricted/data"));
        assert!(r.is_allowed("/open/data"));
    }

    #[test]
    fn rate_limiter_allows_first_request() {
        let l = PerDomainRateLimiter::new(Duration::from_secs(60));
        assert!(l.is_ready("example.com"));
    }

    #[test]
    fn rate_limiter_throttles_after_record() {
        let mut l = PerDomainRateLimiter::new(Duration::from_secs(60));
        l.record("example.com");
        assert!(!l.is_ready("example.com"));
    }

    #[test]
    fn rate_limiter_different_domain_not_throttled() {
        let mut l = PerDomainRateLimiter::new(Duration::from_secs(60));
        l.record("a.com");
        assert!(l.is_ready("b.com"));
    }

    #[test]
    fn strip_html_removes_tags() {
        assert_eq!(strip_html("<h1>Hello</h1><p>World</p>"), "Hello World");
    }

    #[test]
    fn strip_html_collapses_whitespace() {
        assert_eq!(strip_html("<p>  a   b  </p>  <span>c</span>"), "a b c");
    }

    #[test]
    fn extract_title_finds_title() {
        let html = "<html><head><title>My Page</title></head></html>";
        assert_eq!(extract_title(html), "My Page");
    }

    #[test]
    fn extract_title_empty_on_missing() {
        assert_eq!(extract_title("<html><body>no title</body></html>"), "");
    }

    #[test]
    fn needs_playwright_fallback_on_sparse_content() {
        assert!(needs_playwright_fallback(""), "empty triggers fallback");
        assert!(
            needs_playwright_fallback("   "),
            "whitespace triggers fallback"
        );
        assert!(
            needs_playwright_fallback("Too short"),
            "very short triggers fallback"
        );
    }

    #[test]
    fn needs_playwright_fallback_not_on_rich_content() {
        let rich = "This is a full article about Bitcoin and cryptocurrency markets with substantial content. ".repeat(3);
        assert!(!needs_playwright_fallback(&rich));
    }

    #[test]
    fn extract_domain_parses_https() {
        assert_eq!(
            extract_domain("https://www.example.com/path?q=1"),
            Some("www.example.com".into())
        );
    }

    #[test]
    fn extract_domain_parses_http() {
        assert_eq!(
            extract_domain("http://example.com/"),
            Some("example.com".into())
        );
    }
}
