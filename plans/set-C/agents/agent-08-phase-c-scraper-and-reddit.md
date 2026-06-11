# Agent Query — Web Scraper Trie + Reddit Mention Table + Credential Borrow Cleanup
## Covers Issues: #32, #46, #51, #52, #53, #61, #65
## Phase: C
## Estimated Effort: 2–3 days
## Prerequisites: None

---

**How to use this query:** Paste the contents of this file into a new Claude Code session opened in the trading-bot repository root. The agent will implement all fixes listed, verify them, and report completion. Each acceptance criterion has a checkbox — the agent should check them off as they pass.

---

## Background

The web scraper checks robots.txt compliance by iterating a `Vec<String>` of disallowed paths and calling `.starts_with()` on each — O(n) per URL check, with no caching between requests to the same domain. The robots.txt `Vec` is built without capacity hints, causing repeated reallocation. The Reddit collector rebuilds a `HashMap<String, f32>` of mention scores on every post. Account source adapters clone credential strings unnecessarily when the HTTP client would accept a `&str` reference. These are all independent fixes grouped together because they are in the satellite (non-hot-path) collector crates and can be done as a batch.

## Codebase Context

- `crates/collectors/src/web/scraper.rs` — around lines 105–165: robots.txt parsing (lines 105–132) builds Vecs without capacity hints; `is_allowed()` (lines 146–165) does O(n) `.starts_with()` scan. No per-domain caching.
- `crates/collectors/src/social/reddit.rs` — around lines 71–105, `extract_mentions()` builds a fresh `HashMap<String, f32>` on every post (line 71); around line 87–90, `known_instruments` is re-iterated on every post.
- `crates/collectors/src/equity/alpaca_data.rs` — around lines 89–101, `.as_deref().unwrap_or("unknown")` followed by `.to_owned()` clones a static string literal.
- `crates/execution/src/account/alpaca.rs` — around lines 43, 48, 51, credentials cloned from config into a Vec, then individual strings cloned again when building HTTP headers.
- `crates/execution/src/account/kraken.rs` — same credential clone pattern.
- `crates/execution/src/account/kalshi.rs` — same pattern.

## Task

### Fix #52 — Vec::with_capacity for robots.txt parsing

**Problem:** `crates/collectors/src/web/scraper.rs` (around lines 105–132) builds the `disallowed` and `allowed` path Vecs using `Vec::new()` and pushing paths one by one. Each push may reallocate the Vec when it exceeds capacity.

**Solution:** Estimate the number of paths before building the Vecs and pre-allocate with `Vec::with_capacity()`.

**Implementation steps:**

1. Before the parsing loop, count the lines once:
   ```rust
   let line_count = robots_txt_body.lines().count();
   // Rough heuristic: half the lines are Disallow, half are other directives
   let mut disallowed = Vec::with_capacity(line_count / 2 + 4);
   let mut allowed    = Vec::with_capacity(line_count / 4 + 4);
   ```

2. The existing line-by-line parsing loop is otherwise unchanged.

3. This eliminates all reallocation during robots.txt parsing for any realistic robots.txt file.

### Fix #53, #61, #65 — Trie-based robots.txt path matching + per-domain caching

**Problem:** `crates/collectors/src/web/scraper.rs` (around lines 146–165): `is_allowed(path: &str)` iterates `disallowed: Vec<String>` calling `.starts_with()` on each entry — O(n) per URL check, where n is the number of disallowed path prefixes. For some sites n can be in the hundreds. Issues #53, #61, and #65 all refer to this same linear scan and absence of caching.

**Solution:** After parsing, build a trie from the disallowed/allowed path lists for O(path_length) lookup. Cache the parsed `RobotsTxt` per domain so it is only parsed once per scraping session.

**Implementation steps:**

1. Add `radix_trie = "0.2"` to workspace `Cargo.toml`:
   ```toml
   radix_trie = "0.2"
   ```
   Alternatively, implement a minimal prefix trie using `HashMap<String, TrieNode>` within the scraper module if adding a dependency is not desired.

2. Add a `RobotsTxt` struct that holds the built trie:
   ```rust
   pub struct RobotsTxt {
       disallowed: radix_trie::Trie<String, ()>,
       allowed:    radix_trie::Trie<String, ()>,
   }

   impl RobotsTxt {
       pub fn parse(body: &str) -> Self {
           let line_count = body.lines().count();
           let mut disallowed_paths = Vec::with_capacity(line_count / 2 + 4);
           let mut allowed_paths    = Vec::with_capacity(line_count / 4 + 4);
           // ... existing parse logic, building the Vec first ...
           let mut disallowed_trie = radix_trie::Trie::new();
           for path in disallowed_paths {
               disallowed_trie.insert(path, ());
           }
           let mut allowed_trie = radix_trie::Trie::new();
           for path in allowed_paths {
               allowed_trie.insert(path, ());
           }
           Self { disallowed: disallowed_trie, allowed: allowed_trie }
       }

       pub fn is_allowed(&self, path: &str) -> bool {
           // Check allowed first (allowed overrides disallowed for specificity)
           if self.allowed.get_ancestor(path).is_some() {
               return true;
           }
           // If any ancestor prefix is disallowed, path is not allowed
           self.disallowed.get_ancestor(path).is_none()
       }
   }
   ```

3. Add per-domain caching of the parsed `RobotsTxt` to the scraper struct. Use `dashmap::DashMap<String, Arc<RobotsTxt>>` (or `HashMap` if single-threaded):
   ```rust
   pub struct Scraper {
       robots_cache: HashMap<String, Arc<RobotsTxt>>,
       cache_ttl:    Duration,   // default: 24 hours
       // ... other fields ...
   }
   ```

4. In the scraper's URL-check method, look up the domain in `robots_cache` before making a robots.txt HTTP request:
   ```rust
   let domain = extract_domain(&url);
   let robots = match self.robots_cache.get(&domain) {
       Some(cached) => Arc::clone(cached),
       None => {
           let body = self.fetch_robots_txt(&domain).await?;
           let parsed = Arc::new(RobotsTxt::parse(&body));
           self.robots_cache.insert(domain.clone(), Arc::clone(&parsed));
           parsed
       }
   };
   if !robots.is_allowed(url.path()) {
       return Err(ScraperError::Disallowed);
   }
   ```

5. Remove the old linear `Vec::contains` / `.starts_with()` loop from `is_allowed()`. The trie replaces it entirely.

### Fix #51 — Pre-allocated scores table in Reddit mention extraction

**Problem:** `crates/collectors/src/social/reddit.rs` (around lines 71–105): `extract_mentions()` builds a fresh `HashMap<String, f32>` on every post. Over millions of Reddit posts, this is constant allocation churn.

**Solution:** Pre-allocate a `Vec<f32>` of scores parallel to the `known_instruments` list. This Vec is allocated once per `RedditCollector` instance, reset to 0.0 at the start of each `extract_mentions` call, and never heap-allocated during processing.

**Implementation steps:**

1. Add a field to the `RedditCollector` (or wherever `extract_mentions` is called):
   ```rust
   pub struct RedditCollector {
       known_instruments: Vec<String>,
       scores_scratch:    Vec<f32>,   // pre-allocated, reused per post
       // ...
   }
   ```
   In the constructor: `scores_scratch: vec![0.0; known_instruments.len()]`.

2. Change `extract_mentions` to accept a mutable scratch buffer:
   ```rust
   fn extract_mentions<'a>(
       &mut self,
       post_text: &str,
       out_mentions: &mut Vec<(usize, f32)>,  // (instrument_index, score)
   ) {
       // Reset scores
       for s in &mut self.scores_scratch { *s = 0.0; }
       // Scan post_text for each instrument name
       for (i, instrument) in self.known_instruments.iter().enumerate() {
           if post_text.contains(instrument.as_str()) {
               self.scores_scratch[i] += 1.0;  // or more complex scoring
           }
       }
       // Collect non-zero results
       out_mentions.clear();
       for (i, &score) in self.scores_scratch.iter().enumerate() {
           if score > 0.0 {
               out_mentions.push((i, score));
           }
       }
   }
   ```

3. Remove the `HashMap<String, f32>` construction from `extract_mentions`. The output is now a `Vec<(usize, f32)>` of (index, score) pairs for non-zero instruments.

### Fix #46 — &'static str for collector field defaults

**Problem:** `crates/collectors/src/equity/alpaca_data.rs` (around lines 89–101): patterns like `.as_deref().unwrap_or("unknown").to_owned()` clone a static string literal `"unknown"` into a heap-allocated `String` unnecessarily.

**Solution:** Avoid calling `.to_owned()` on the result of `unwrap_or("unknown")`. If the consuming code needs an owned `String`, change it to accept `&str` or `Cow<'static, str>`.

**Implementation steps:**

1. Find all patterns of the form: `field.as_deref().unwrap_or("unknown").to_owned()` in `crates/collectors/src/equity/alpaca_data.rs`.

2. If the result is assigned to a `String` field in a struct, change that field to `Cow<'static, str>`:
   ```rust
   use std::borrow::Cow;
   // Before:
   exchange: msg.exchange.as_deref().unwrap_or("unknown").to_owned(),
   // After:
   exchange: msg.exchange.as_deref()
       .map(|s| Cow::Owned(s.to_owned()))
       .unwrap_or(Cow::Borrowed("unknown")),
   ```
   When the field is the literal "unknown", no allocation occurs. When the field has a real value, it allocates once (unavoidable since we own the data).

3. Alternatively, if the consuming code only needs `&str` (e.g., it passes to a format! or log call), change the signature to return `&str` directly from a helper:
   ```rust
   fn exchange_str(msg: &AlpacaMessage) -> &str {
       msg.exchange.as_deref().unwrap_or("unknown")
   }
   ```
   No allocation at all.

### Fix #32 — Borrow credentials in account adapters

**Problem:** `crates/execution/src/account/alpaca.rs` (around lines 43, 48, 51): API key and secret credentials are cloned from config into local variables, then cloned again when building HTTP request headers. `reqwest::RequestBuilder::header()` accepts `&str`, so no owned `String` is needed.

**Solution:** Store parsed credentials as `Arc<str>` at startup. Pass `&str` to HTTP request builders — no clone needed.

**Implementation steps:**

1. In the `AlpacaAccountSource` struct, change credential fields:
   ```rust
   // Before:
   api_key: String,
   api_secret: String,
   // After:
   api_key: Arc<str>,
   api_secret: Arc<str>,
   ```

2. In the constructor, convert `String` to `Arc<str>` once at startup:
   ```rust
   api_key: Arc::from(config.api_key.as_str()),
   api_secret: Arc::from(config.api_secret.as_str()),
   ```

3. In HTTP request construction, pass `&*self.api_key` (deref to `&str`) to `reqwest`:
   ```rust
   .header("APCA-API-KEY-ID", &*self.api_key)
   .header("APCA-API-SECRET-KEY", &*self.api_secret)
   ```
   This passes a `&str` reference — no clone.

4. Remove the intermediate `Vec<(String, String)>` of header pairs if one exists — build headers directly from `Arc<str>` deref.

5. Apply the same pattern to:
   - `crates/execution/src/account/kraken.rs`
   - `crates/execution/src/account/kalshi.rs`

**Acceptance test:**
- Write a unit test for `RobotsTxt::parse()` with a known robots.txt body. Test that `is_allowed("/public/page")` returns true and `is_allowed("/private/admin")` returns false.
- Write a unit test for `RedditCollector::extract_mentions()` that processes 1,000 synthetic posts and verifies the scores_scratch Vec is reused (same memory address each call).
- `cargo test` passes for all affected crates.

## Overall Acceptance Criteria
- [ ] `RobotsTxt::is_allowed()` uses trie lookup, not Vec iteration (no `.starts_with()` loop)
- [ ] `RobotsTxt` structs are cached per domain in the scraper (at most one parse per domain per session)
- [ ] `Vec::with_capacity` used for robots.txt path list construction
- [ ] Reddit `extract_mentions()` reuses a pre-allocated `scores_scratch: Vec<f32>` field
- [ ] No `HashMap<String, f32>` construction in Reddit hot path
- [ ] No `.to_owned()` on static string defaults (`"unknown"`) in alpaca_data.rs
- [ ] Credentials stored as `Arc<str>` in account adapters; `&str` passed to reqwest headers
- [ ] `cargo test` passes for all affected crates

## Files to Touch
- `crates/collectors/src/web/scraper.rs` — trie-based RobotsTxt; per-domain cache; Vec::with_capacity
- `crates/collectors/src/social/reddit.rs` — pre-allocated scores_scratch Vec; remove HashMap per post
- `crates/collectors/src/equity/alpaca_data.rs` — fix static default string clone
- `crates/execution/src/account/alpaca.rs` — Arc<str> credentials; &str to reqwest
- `crates/execution/src/account/kraken.rs` — same credential pattern
- `crates/execution/src/account/kalshi.rs` — same credential pattern
- `Cargo.toml` — add `radix_trie = "0.2"` to workspace dependencies
