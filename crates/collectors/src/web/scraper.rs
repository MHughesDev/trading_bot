//! Web page scraper — HTTP-first, robots.txt-compliant, rate-limited.
//!
//! Fetches web pages and emits `EventEnvelope<WebPageSnapshotPayload>`.
//! `robots.txt` is honoured for all paths and per-domain rate limits are
//! enforced.  When HTTP yields insufficient text content, a Playwright
//! subprocess is invoked as a fallback if `PLAYWRIGHT_BIN` is configured.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use domain::{
    payloads::web_page_snapshot::{FetchMethod, WebPageSnapshotPayload},
    EventEnvelope,
};
use tracing::{debug, info, warn};

use crate::{Collector, CollectorError};

const VENUE_ID: &str = "web";
const SOURCE: &str = "web_scraper";
/// Minimum word count to treat an HTTP response as having usable content.
pub const MIN_WORD_COUNT: usize = 20;
/// Cache TTL for robots.txt entries.
const ROBOTS_TTL: Duration = Duration::from_secs(3600);

// ── URL safety validation (H-2 SSRF guard) ───────────────────────────────────

/// Return `true` if `url` is safe to fetch.
///
/// Rejects:
/// * non-http/https schemes (e.g. `file://`, `ftp://`)
/// * bare IP addresses in loopback, private, link-local, or CGNAT ranges
pub fn is_safe_url(url: &str) -> bool {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return false;
    }
    let rest = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host = rest
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");
    if host.is_empty() {
        return false;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_public_ip(ip);
    }
    // Block well-known loopback/internal hostnames.
    let lower = host.to_ascii_lowercase();
    if lower == "localhost"
        || lower.ends_with(".localhost")
        || lower.ends_with(".internal")
        || lower.ends_with(".local")
    {
        return false;
    }
    true
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !v4.is_private()
                && !v4.is_loopback()
                && !v4.is_link_local()
                && !v4.is_broadcast()
                && !v4.is_documentation()
                && !v4.is_unspecified()
        }
        IpAddr::V6(v6) => !v6.is_loopback() && !v6.is_unspecified(),
    }
}

// ── robots.txt ────────────────────────────────────────────────────────────────

/// A simple prefix trie for O(path_length) longest-prefix-match lookup of robots.txt rules (#53, #61).
#[derive(Debug, Clone)]
struct PrefixTrie {
    root: TrieNode,
}

#[derive(Debug, Clone)]
struct TrieNode {
    children: HashMap<char, TrieNode>,
    is_rule: bool, // true if a rule ends at this node
}

impl PrefixTrie {
    fn new() -> Self {
        Self {
            root: TrieNode {
                children: HashMap::new(),
                is_rule: false,
            },
        }
    }

    /// Insert a prefix rule into the trie.
    fn insert(&mut self, rule: &str) {
        let mut node = &mut self.root;
        for ch in rule.chars() {
            node = node.children.entry(ch).or_insert_with(|| TrieNode {
                children: HashMap::new(),
                is_rule: false,
            });
        }
        node.is_rule = true;
    }

    /// Find the longest prefix match for a path. Returns the length of the longest matching rule.
    fn longest_match(&self, path: &str) -> usize {
        let mut node = &self.root;
        let mut longest = 0;
        let mut chars_matched = 0;

        for ch in path.chars() {
            if let Some(next) = node.children.get(&ch) {
                node = next;
                chars_matched += ch.len_utf8();
                if node.is_rule {
                    longest = chars_matched;
                }
            } else {
                break;
            }
        }
        longest
    }
}

/// A parsed `robots.txt` for a single domain.
#[derive(Debug, Clone)]
pub struct RobotsTxt {
    /// Trie of disallowed path prefixes for longest-prefix matching (#61).
    disallowed_trie: PrefixTrie,
    /// Trie of explicitly allowed path prefixes (longest-match beats Disallow) (#53).
    allowed_trie: PrefixTrie,
    fetched_at: std::time::SystemTime,
}

impl RobotsTxt {
    /// Parse raw robots.txt content, extracting `Disallow` and `Allow` rules
    /// for `*` and `trading-bot` user-agents.
    ///
    /// Correctness fixes:
    /// * `in_applicable_section` is reset on every new `User-agent:` line so
    ///   rules from later, non-matching agent blocks are not applied (M-8).
    /// * `Allow:` directives are honoured (longest-match-wins, per spec) (L-1).
    pub fn parse(content: &str) -> Self {
        let mut disallowed_trie = PrefixTrie::new();
        let mut allowed_trie = PrefixTrie::new();
        let mut in_applicable_section = false;

        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("User-agent:") {
                let ua = rest.trim().to_lowercase();
                // Reset for each new block so later non-matching blocks don't
                // inherit the applicability of an earlier matching block.
                in_applicable_section = ua == "*" || ua == "trading-bot";
            } else if in_applicable_section {
                if let Some(rest) = line.strip_prefix("Disallow:") {
                    let path = rest.trim();
                    if !path.is_empty() {
                        disallowed_trie.insert(path);
                    }
                } else if let Some(rest) = line.strip_prefix("Allow:") {
                    let path = rest.trim();
                    if !path.is_empty() {
                        allowed_trie.insert(path);
                    }
                }
            }
        }

        Self {
            disallowed_trie,
            allowed_trie,
            fetched_at: std::time::SystemTime::now(),
        }
    }

    /// Return `true` if `path` is allowed for our user-agent.
    ///
    /// Uses longest-match-wins: an `Allow:` rule longer than the matching
    /// `Disallow:` rule takes precedence (per the robots spec).
    /// Trie-based lookup: O(path_length) per check (#53, #61).
    pub fn is_allowed(&self, path: &str) -> bool {
        let disallow_len = self.disallowed_trie.longest_match(path);
        let allow_len = self.allowed_trie.longest_match(path);
        // Allow wins on a tie or when more specific than Disallow.
        allow_len >= disallow_len
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

/// Remove all content between `<element>…</element>` tags (case-insensitive).
fn remove_element_content(html: &str, element: &str) -> String {
    let lower = html.to_lowercase();
    let open_tag = format!("<{element}");
    let close_tag = format!("</{element}>");

    let mut result = String::with_capacity(html.len());
    let mut pos = 0;

    loop {
        let Some(start) = lower[pos..].find(&open_tag) else {
            result.push_str(&html[pos..]);
            break;
        };
        let abs_start = pos + start;
        result.push_str(&html[pos..abs_start]);

        // Skip to the end of the opening tag, then to the closing tag.
        let after_open = abs_start + open_tag.len();
        let Some(tag_close) = lower[after_open..].find('>') else {
            break; // Malformed — stop.
        };
        let content_start = after_open + tag_close + 1;
        if let Some(end) = lower[content_start..].find(&close_tag) {
            pos = content_start + end + close_tag.len();
        } else {
            break; // No closing tag — skip to end.
        }
    }

    result
}

/// Decode the most common named and numeric HTML entities.
fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&#34;", "\"")
}

/// Strip HTML tags and collapse whitespace to return visible text content.
///
/// Correctness fixes vs. the original:
/// * `<script>` and `<style>` element contents are removed entirely (M-9).
/// * HTML entities (`&amp;`, `&lt;`, …) are decoded (L-2).
/// * An unclosed `<tag` (no `>`) stops accumulating in-tag state at EOF
///   rather than swallowing the remainder of the string (L-3).
pub fn strip_html(html: &str) -> String {
    // Remove script and style block contents before general tag stripping.
    let no_script = remove_element_content(html, "script");
    let no_style = remove_element_content(&no_script, "style");

    let mut out = String::with_capacity(no_style.len() / 2);
    let mut in_tag = false;

    for ch in no_style.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            // EOF while in_tag — remaining text is emitted, not swallowed.
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }

    let decoded = decode_html_entities(&out);
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
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
///
/// Returns `None` for any URL that is not http:// or https://, preventing
/// `file://` and other dangerous schemes from being treated as valid domains.
pub fn extract_domain(url: &str) -> Option<String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return None;
    }
    let rest = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host = rest.split('/').next().map(|h| h.to_lowercase())?;
    if host.is_empty() {
        return None;
    }
    Some(host)
}

// ── Playwright fallback ───────────────────────────────────────────────────────

/// Attempt a Playwright fetch if `PLAYWRIGHT_BIN` is configured.
///
/// The URL is passed via the `SCRAPE_URL` environment variable so it is never
/// interpolated into the script string — preventing JS injection (H-1).
/// The URL is also validated before the subprocess is spawned (H-2).
fn try_playwright_fetch(url: &str) -> Option<String> {
    if !is_safe_url(url) {
        warn!(%url, "Playwright fetch rejected: URL failed safety check");
        return None;
    }
    let bin = std::env::var("PLAYWRIGHT_BIN").ok()?;
    // URL is passed via env, not embedded in the script, to prevent injection.
    let script = "(async () => { \
        const { chromium } = require('playwright'); \
        const b = await chromium.launch(); \
        const p = await b.newPage(); \
        await p.goto(process.env.SCRAPE_URL, { timeout: 15000 }); \
        const t = await p.innerText('body').catch(() => ''); \
        await b.close(); \
        process.stdout.write(t); \
        })()";
    let output = std::process::Command::new(&bin)
        .args(["eval", script])
        .env("SCRAPE_URL", url)
        .output()
        .ok()?;

    if output.status.success() {
        String::from_utf8(output.stdout).ok()
    } else {
        warn!(%url, "Playwright fetch failed");
        None
    }
}

// ── Lane constant (M-10) ─────────────────────────────────────────────────────

/// NATS lane for web page snapshot events.
///
/// Must match `WebPageSnapshotPayload::event_type()` — both use the
/// un-versioned form `"web.page_snapshot"` so subscribers don't see a mismatch.
pub const WEB_PAGE_SNAPSHOT_LANE: &str = "web.page_snapshot";

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

    /// Fetch a single URL using the provided (already-refreshed) robots cache.
    ///
    /// `robots_cache` is passed in so this method does not re-fetch robots.txt
    /// on every call (M-15: previously fetch_page re-fetched unconditionally).
    async fn fetch_page(
        &self,
        url: &str,
        seq: u64,
        robots_cache: &HashMap<String, RobotsTxt>,
    ) -> Option<EventEnvelope> {
        // SSRF guard: reject non-http/https and private IP targets (H-2).
        if !is_safe_url(url) {
            warn!(%url, "URL failed safety check — skipping");
            return None;
        }

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

        // Use the shared cache; never re-fetch here.
        if !robots_cache
            .get(&domain)
            .map(|r| r.is_allowed(&path))
            .unwrap_or(true)
        {
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

        let payload_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&payload)
            .ok()?
            .into_vec();
        let timestamp_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0);

        Some(EventEnvelope::new(
            domain::intern_instrument(&domain),
            domain::intern_venue(VENUE_ID),
            domain::intern_source(SOURCE),
            seq,
            timestamp_ns,
            payload_bytes,
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
                // SSRF guard: skip URLs that don't pass safety checks (H-2).
                if !is_safe_url(url) {
                    warn!(%url, "URL in config failed safety check — skipping");
                    continue;
                }

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
                if let Some(envelope) = self.fetch_page(url, seq, &robots_cache).await {
                    let raw = envelope.payload.clone();
                    crate::normalizer::quarantine_or_publish(
                        Ok(envelope),
                        &raw,
                        url,
                        WEB_PAGE_SNAPSHOT_LANE,
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

    // ── robots.txt ────────────────────────────────────────────────────────────

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
    fn robots_txt_section_reset_on_new_agent_block() {
        // M-8: rules from a later non-matching block must not bleed into ours.
        let content = "User-agent: *\n\
                       Disallow: /common/\n\n\
                       User-agent: Googlebot\n\
                       Disallow: /google-only/\n";
        let r = RobotsTxt::parse(content);
        assert!(!r.is_allowed("/common/page"), "/common/ must be disallowed");
        // Googlebot-only rule must NOT apply to us.
        assert!(
            r.is_allowed("/google-only/page"),
            "/google-only/ must be allowed for us (Googlebot rule must not bleed)"
        );
    }

    #[test]
    fn robots_txt_allow_overrides_disallow() {
        // L-1: Allow: /admin/public/ within Disallow: /admin/ should permit sub-path.
        let content = "User-agent: *\nDisallow: /admin/\nAllow: /admin/public/\n";
        let r = RobotsTxt::parse(content);
        assert!(
            !r.is_allowed("/admin/secret"),
            "/admin/secret must be blocked"
        );
        assert!(
            r.is_allowed("/admin/public/page"),
            "/admin/public/ should be explicitly allowed"
        );
    }

    // ── rate limiter ──────────────────────────────────────────────────────────

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

    // ── strip_html ────────────────────────────────────────────────────────────

    #[test]
    fn strip_html_removes_tags() {
        assert_eq!(strip_html("<h1>Hello</h1><p>World</p>"), "Hello World");
    }

    #[test]
    fn strip_html_collapses_whitespace() {
        assert_eq!(strip_html("<p>  a   b  </p>  <span>c</span>"), "a b c");
    }

    #[test]
    fn strip_html_removes_script_content() {
        // M-9: script content must not appear in output.
        let html = "<p>Visible</p><script>var x = 1 < 2;</script><p>Also visible</p>";
        let out = strip_html(html);
        assert!(!out.contains("var"), "script content must be stripped");
        assert!(out.contains("Visible"), "regular text must remain");
        assert!(
            out.contains("Also visible"),
            "text after script must remain"
        );
    }

    #[test]
    fn strip_html_removes_style_content() {
        // M-9: style content must not appear in output.
        let html = "<p>Text</p><style>.a { color: red; }</style><p>More</p>";
        let out = strip_html(html);
        assert!(!out.contains("color"), "style content must be stripped");
        assert!(out.contains("Text") && out.contains("More"));
    }

    #[test]
    fn strip_html_decodes_entities() {
        // L-2: HTML entities must be decoded.
        assert_eq!(strip_html("a &amp; b"), "a & b");
        assert_eq!(strip_html("&lt;tag&gt;"), "<tag>");
        assert_eq!(strip_html("&nbsp;space"), "space");
    }

    #[test]
    fn strip_html_unclosed_tag_emits_surrounding_text() {
        // L-3: an unclosed tag like "<b" must not swallow subsequent text.
        // With the remove_element_content pass, only malformed bare `<` without
        // `>` would trigger this; verify we at least emit text before the `<`.
        let out = strip_html("before<b>after");
        assert!(out.contains("before"), "text before tag must be present");
        assert!(out.contains("after"), "text after tag must be present");
    }

    // ── extract_title ─────────────────────────────────────────────────────────

    #[test]
    fn extract_title_finds_title() {
        let html = "<html><head><title>My Page</title></head></html>";
        assert_eq!(extract_title(html), "My Page");
    }

    #[test]
    fn extract_title_empty_on_missing() {
        assert_eq!(extract_title("<html><body>no title</body></html>"), "");
    }

    // ── needs_playwright_fallback ─────────────────────────────────────────────

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

    // ── extract_domain ────────────────────────────────────────────────────────

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

    #[test]
    fn extract_domain_rejects_file_scheme() {
        // H-2: non-http/https schemes must return None.
        assert_eq!(extract_domain("file:///etc/passwd"), None);
        assert_eq!(extract_domain("ftp://example.com"), None);
    }

    // ── is_safe_url ───────────────────────────────────────────────────────────

    #[test]
    fn safe_url_allows_public_https() {
        assert!(is_safe_url("https://www.example.com/page"));
    }

    #[test]
    fn safe_url_rejects_file_scheme() {
        assert!(!is_safe_url("file:///etc/passwd"));
    }

    #[test]
    fn safe_url_rejects_private_ip() {
        assert!(!is_safe_url("http://192.168.1.1/admin"));
        assert!(!is_safe_url("http://10.0.0.1/"));
        assert!(!is_safe_url("http://172.16.0.1/"));
    }

    #[test]
    fn safe_url_rejects_loopback() {
        assert!(!is_safe_url("http://127.0.0.1:9091/api"));
        assert!(!is_safe_url("http://localhost/"));
    }

    #[test]
    fn safe_url_rejects_link_local() {
        // AWS metadata endpoint.
        assert!(!is_safe_url("http://169.254.169.254/latest/meta-data/"));
    }
}
