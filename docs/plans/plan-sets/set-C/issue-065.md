# Issue #065 — Web scraper: multiple string-based path lookups per fetch

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | F |
| Pattern | Search |
| Quick Win | No |
| Latency Impact | 20–30 string operations per fetch |
| Location | `crates/collectors/src/web/scraper.rs:39,45-51,114-130,146-150,157-165` |

## Problem
URL parsing, robots.txt parsing, path checking, and content extraction are all performed via repeated string operations (split, trim, starts_with, ends_with) on every fetch. No caching of parsed robots.txt or parsed URLs. This adds 20-30 string operations per fetch, none of which are cached.

## Root Cause
The scraper treats each fetch as stateless — robots.txt is re-parsed, URLs are re-split, and path rules are re-checked from scratch on every request. There is no per-domain cache for parsed robots.txt or for resolved URL structure.

## Implementation Plan
### Step 1 — Cache parsed robots.txt per domain
Store parsed robots.txt rules (as a trie, from #53) in a `HashMap<Domain, Arc<RulesTrie>>` on the scraper. Fetch and parse robots.txt once per domain; reuse the parsed trie for all subsequent requests to that domain. Implement a TTL (e.g., 24 hours) for cache invalidation.

### Step 2 — Cache parsed URLs
Use the `url` crate for URL parsing. Parse once per fetch; pass the parsed `Url` struct to all downstream functions instead of re-splitting the raw string.

### Step 3 — Consolidate with #52, #53, #61
All web scraper string optimization issues (#52, #53, #61, #65) should be addressed together in a single refactoring PR:
1. #52: Vec::with_capacity for parsed rules
2. #53 + #61: trie for path matching
3. #65: domain-level robots.txt cache + URL parsing cache

### Step 4 — Reduce per-fetch string operations
After caching, per-fetch string operations should be:
- URL parsed once → `Url` struct → no further string splits
- Domain looked up in robots.txt cache → trie lookup O(path_length)
- Total: ~3 operations instead of 20-30

## Acceptance Criteria
- [ ] robots.txt parsed at most once per domain per 24 hours
- [ ] URL parsed once per fetch (no repeated string splits)
- [ ] Per-fetch string operations ≤ 5 (domain lookup + trie path check + URL parse)
- [ ] Scraper integration test: fetches to same domain reuse robots.txt cache
- [ ] Cache eviction test: stale robots.txt cache is refreshed after TTL

## Files to Change
- `crates/collectors/src/web/scraper.rs` — add domain-level robots.txt cache; use url crate for URL parsing; consolidate with #52, #53, #61 at lines 39, 45-51, 114-130, 146-150, 157-165
