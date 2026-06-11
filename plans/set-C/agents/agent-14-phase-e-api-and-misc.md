# Agent Query — API/WS Hygiene Sweep: Binary WS Frames, Clone Removal, Format Deferral, Manifest Dedup at Load Time
## Covers Issues: #14, #15, #16, #19, #20, #23, #33, #38, #39, #41, #42, #44, #49, #50, #56, #57, #58, #59, #62, #67, #68
## Phase: E
## Estimated Effort: 1–2 days (can be done in parallel by splitting groups)
## Prerequisites: None (all independent)

---

**How to use this query:** Paste the contents of this file into a new Claude Code session opened in the trading-bot repository root. The agent will implement all fixes listed, verify them, and report completion. Each acceptance criterion has a checkbox — the agent should check them off as they pass.

---

## Background

This is a batch of 21 independent hygiene fixes across the API, strategy-runtime, collectors, graph, and semantic crates. Each is a small change (< 50 lines of code) that eliminates unnecessary allocations, string clones, or eager string formatting. None are individually on the microsecond hot path, but together they eliminate allocation churn throughout the system and reduce steady-state memory pressure. They are grouped here because they can be implemented and reviewed together without dependencies on each other or on the Phase A/B/C/D work.

## Codebase Context

Key files and their issues:
- `crates/api/src/ws/live.rs` — line 141: `serde_json::to_string` per WS frame (#14); lines 110, 113: `panel_id.clone(), instrument_id.clone()` (#15)
- `crates/api/src/rollup/mod.rs` — lines 72–89: 4-pass HashMap rebuild (#16)
- `crates/api/src/routes/dashboard.rs` — line 41: eager `format!("error: {e}")` (#20)
- `crates/api/src/routes/strategies.rs` — lines 241–242: `format!("{:?}").to_lowercase()` for enum serialization (#38)
- `crates/api/src/routes/venue_health.rs` — lines 129–140: eager format on each error (#49)
- `crates/strategy-runtime/src/runtime.rs` — lines 117, 140: `Arc<str>` for instrument_id keys (#19)
- `crates/strategy-runtime/src/manifest.rs` — lines 94–127: HashSet rebuild and tree walk on every `compile_manifest()` call (#56, #57, #68)
- `crates/strategy-runtime/tests/manifest_compile.rs` — line 80: unnecessary clone in test (#23)
- `crates/graph/src/populate.rs` — lines 80–90: `serde_json::to_value(a).as_str().unwrap().to_owned()` for enum name (#58); `.as_key().to_owned()` collect (#59)
- `crates/graph/src/schema.rs` — `Vec<Vec<T>>` nested structure (#44)
- `crates/semantic/src/lib.rs` — line 90: `SOCIAL_COLLECTION.to_owned()` (#62)
- `crates/collectors/src/social/reddit.rs` — lines 87–90: repeated iteration over known_instruments (#67); lines 131, 134–135: `title.clone()` (#39)
- `crates/execution/src/account/kalshi.rs` — line 115: no `with_capacity` and no `#[serde(borrow)]` (#33)
- `crates/venue-router/src/lifecycle.rs` — lines 78–80: `.to_owned()` on key components (#42)
- `crates/reconciliation/src/positions.rs` — lines 35–36: `.replace()` per comparison instead of at load time (#41)
- `crates/event-bus/src/nats.rs` — lines 51–81: two passes over 12-item lanes array (#50)

## Task

### Group 1: API and WS frame encoding

**Fix #14 — Binary encoding for WS messages:**

`crates/api/src/ws/live.rs:141` — `serde_json::to_string(&message)` is called for every outgoing WS frame. JSON is verbose and slow to encode.

1. Add `postcard = { version = "1", features = ["alloc"] }` to workspace `Cargo.toml`.
2. Change the WS send path:
   ```rust
   // Before:
   let json = serde_json::to_string(&message)?;
   ws_sink.send(Message::Text(json)).await?;
   // After:
   let bytes = postcard::to_allocvec(&message)?;
   ws_sink.send(Message::Binary(bytes)).await?;
   ```
3. Keep JSON for the initial connection handshake and state-sync message (send once on connect). All subsequent real-time update frames use binary postcard encoding.
4. Add `#[derive(serde::Serialize, serde::Deserialize)]` to all WS message types (already present for JSON; postcard uses the same serde derives).
5. Update the React frontend protocol documentation (if any) to note the binary frame format. The frontend must use `postcard` WASM or a compatible decoder.

**Fix #15 — Panel/instrument IDs by reference:**

`crates/api/src/ws/live.rs` (around lines 110, 113) — `panel_id.clone(), instrument_id.clone()` are passed to a function that only needs to read them.

1. Change the function signature to accept `panel_id: &str, instrument_id: &str`.
2. At the call site, pass `&panel_id, &instrument_id` — no clone.
3. If the function stores these values, it can call `.to_owned()` internally, but only once on the storage path, not on the read path.

**Fix #16 — Single-pass rollup grouping:**

`crates/api/src/rollup/mod.rs` (around lines 72–89) — rebuilds a `HashMap` in 4 passes to group and aggregate rollup data.

1. Replace the 4-pass algorithm with a single-pass using `HashMap::entry()`:
   ```rust
   let mut groups: HashMap<GroupKey, Accumulator> = HashMap::new();
   for item in &items {
       groups.entry(item.group_key()).or_insert_with(Accumulator::new).add(item);
   }
   let result: Vec<RollupRow> = groups.into_values().map(|acc| acc.finish()).collect();
   ```
2. This reduces allocations by 3× and improves cache locality.

### Group 2: Error formatting deferral

**Fix #20 + #49 — Defer error formatting:**

`crates/api/src/routes/dashboard.rs:41` and `crates/api/src/routes/venue_health.rs:129-140` — patterns like `format!("error: {e}")` or `.map_err(|e| format!("...: {e}"))` allocate a String even when the error is later discarded or only formatted for logging.

1. Add `thiserror = "1"` to workspace `Cargo.toml` if not already present.
2. Define typed error enums using `#[derive(thiserror::Error)]`. The `#[error("...")]` attribute provides lazy formatting — the string is only constructed when `Display::fmt` is called (i.e., when the error is actually printed).
3. Replace `format!(...)` error construction with the typed enum:
   ```rust
   // Before:
   .map_err(|e| format!("database error: {e}"))
   // After:
   .map_err(DatabaseError::Query)   // no allocation at construction
   ```
4. Apply to all `.map_err(|e| format!(...))` patterns in the routes files.

**Fix #38 — Enum serialization without Debug format:**

`crates/api/src/routes/strategies.rs` (around lines 241–242) — `format!("{:?}", enum_value).to_lowercase()` allocates a String to get the enum's name as a lowercase string.

1. Add an `as_str(&self) -> &'static str` method to the affected enum:
   ```rust
   impl OrderStatus {
       pub fn as_str(&self) -> &'static str {
           match self {
               Self::Open => "open",
               Self::Filled => "filled",
               Self::Cancelled => "cancelled",
           }
       }
   }
   ```
2. Replace `format!("{:?}", status).to_lowercase()` with `status.as_str()`. No allocation.
3. Alternatively, add `#[serde(rename_all = "lowercase")]` to the enum and use `serde_json::to_value(&status)?.as_str()?.to_owned()` — but this still allocates. The `as_str()` method is the zero-allocation solution.

### Group 3: Strategy-runtime manifest

**Fix #56 + #57 + #68 — Move dedup to definition load:**

`crates/strategy-runtime/src/manifest.rs` (around lines 94–127): `compile_manifest()` rebuilds `seen_lanes: HashSet<String>` and `seen_features: HashSet<String>` by walking the entire strategy definition tree on every call. All three issues (#56, #57, #68) refer to this same pattern — distinct entry points, same root cause.

1. Find the `StrategyDefinition` struct (or equivalent). Add pre-deduplicated fields:
   ```rust
   pub struct StrategyDefinition {
       // ... existing fields ...
       // Added at parse time:
       pub unique_lanes: Vec<Arc<str>>,       // deduplicated lane list
       pub unique_features: Vec<Arc<str>>,    // deduplicated feature list
   }
   ```

2. At definition-load/parse time (when YAML/JSON is first deserialized into `StrategyDefinition`), compute and store the deduplicated lists:
   ```rust
   fn parse_definition(raw: &str) -> Result<StrategyDefinition> {
       let mut def: StrategyDefinition = serde_json::from_str(raw)?;
       // Dedup lanes
       let mut seen_lanes = HashSet::new();
       def.unique_lanes = def.nodes.iter()
           .flat_map(|n| n.required_lanes())
           .filter(|lane| seen_lanes.insert(lane.clone()))
           .map(Arc::from)
           .collect();
       // Dedup features
       let mut seen_features = HashSet::new();
       def.unique_features = def.nodes.iter()
           .flat_map(|n| n.required_features())
           .filter(|f| seen_features.insert(f.clone()))
           .map(Arc::from)
           .collect();
       Ok(def)
   }
   ```

3. In `compile_manifest()`, replace the HashSet rebuild and tree walk with reads from `def.unique_lanes` and `def.unique_features`:
   ```rust
   pub fn compile_manifest(def: &StrategyDefinition) -> Manifest {
       // No HashSet, no tree walk — just read pre-computed fields
       Manifest {
           lanes: def.unique_lanes.clone(),    // Vec<Arc<str>> clone = vec of refcount bumps
           features: def.unique_features.clone(),
           // ...
       }
   }
   ```

4. For fix #57 specifically: feature names stored in the definition should use `Arc<str>`, not `String`, so `.clone()` in the HashSet insert is a refcount bump. The `Arc::from` conversion in step 2 handles this.

**Fix #23 — Test manifest: avoid clone:**

`crates/strategy-runtime/tests/manifest_compile.rs:80` — a test constructs a manifest and then unnecessarily clones it before passing to a function.

1. Refactor the test to construct the manifest directly in the correct form:
   ```rust
   // Before:
   let manifest = build_test_manifest();
   assert_eq!(compile_manifest(&manifest.clone()), expected);  // ← unnecessary clone
   // After:
   let manifest = build_test_manifest();
   assert_eq!(compile_manifest(&manifest), expected);           // ← pass by reference
   ```
2. Change `compile_manifest` to accept `&StrategyDefinition` (already the case after fix #56).

### Group 4: Graph, semantic, and collector misc

**Fix #58 — AssetClass::as_str():**

`crates/graph/src/populate.rs` (around lines 80–85) — `serde_json::to_value(asset_class).as_str().unwrap().to_owned()` to get the asset class name as a string. This serializes the enum to JSON just to extract its string name.

1. Add `as_str(&self) -> &'static str` to `AssetClass`:
   ```rust
   impl AssetClass {
       pub fn as_str(&self) -> &'static str {
           match self {
               Self::Equity => "equity",
               Self::Crypto => "crypto",
               Self::Futures => "futures",
               Self::Options => "options",
               Self::Fx => "fx",
               // ...
           }
       }
   }
   ```
2. Replace `serde_json::to_value(a).as_str().unwrap().to_owned()` with `a.as_str()`. Zero allocation.

**Fix #59 — Collect references for DataType:**

`crates/graph/src/populate.rs` (around line 90) — `.as_key().to_owned()` collects owned Strings where `&str` references would suffice.

1. Change the `.as_key()` method (or the collection) to return `&'static str` if the values are from a fixed enum. Collect `&'static str` instead of `String`:
   ```rust
   // Before:
   let keys: Vec<String> = data_types.iter().map(|d| d.as_key().to_owned()).collect();
   // After:
   let keys: Vec<&'static str> = data_types.iter().map(|d| d.as_key()).collect();
   ```
2. If the collection outlives the data_types slice and `&'static str` is not available, use `Arc<str>` instead of `String`.

**Fix #44 — Flatten nested Vec<Vec<>> in schema:**

`crates/graph/src/schema.rs` — uses `Vec<Vec<T>>` to represent grouped data, causing per-group heap allocation.

1. Replace with CSR (Compressed Sparse Row) flat layout:
   ```rust
   // Before:
   pub groups: Vec<Vec<SchemaEntry>>,
   // After:
   pub entries: Vec<SchemaEntry>,   // flat, all groups concatenated
   pub offsets: Vec<usize>,         // offsets[i]..offsets[i+1] = group i's entries
   ```
   This is a single Vec allocation for all entries plus a small Vec for offsets, regardless of group count.
2. Update all iteration sites: instead of `for group in &schema.groups { for entry in group { ... } }`, use `for i in 0..schema.offsets.len()-1 { for entry in &schema.entries[schema.offsets[i]..schema.offsets[i+1]] { ... } }`.

**Fix #62 — Static str for Milvus config:**

`crates/semantic/src/lib.rs:90` — `SOCIAL_COLLECTION.to_owned()` converts a `&'static str` constant to an owned `String` unnecessarily.

1. Check the type expected by the Milvus client call at this point.
2. If the API accepts `&str` or `impl AsRef<str>`: pass `SOCIAL_COLLECTION` directly without `.to_owned()`.
3. If the API requires `String`: use `Cow::Borrowed(SOCIAL_COLLECTION)` and only materialize to String if necessary, or accept the single one-time allocation at startup (not hot path).

**Fix #39 — Reddit title as_deref:**

`crates/collectors/src/social/reddit.rs` (around lines 131, 134–135) — `title.clone().map(|t| t + suffix)` clones the title Option<String> and then appends.

1. Replace with `as_deref()` to borrow without cloning:
   ```rust
   // Before:
   let full_title = title.clone().map(|t| t + " [Reddit]");
   // After:
   let full_title = title.as_deref().map(|t| format!("{t} [Reddit]"));
   ```
   Only one allocation (the format output) instead of two (clone + concatenation).

**Fix #67 — Reddit: Arc<HashSet> for known instruments:**

`crates/collectors/src/social/reddit.rs` (around lines 87–90) — `known_instruments` is iterated on every post for mention detection. If it is re-fetched or re-constructed per batch, wrap in `Arc<HashSet<String>>`.

1. If `known_instruments` is currently a `Vec<String>` rebuilt per-batch, change to `Arc<HashSet<String>>` built once at collector initialization.
2. Share the `Arc<HashSet<String>>` across all Reddit posts in a batch — no clone of the set itself.
3. The `HashSet` provides O(1) lookup for "is this word a known instrument?" checks.

**Fix #33 — Account source borrow from JSON:**

`crates/execution/src/account/kalshi.rs:115` — response parsing allocates Strings for fields that could be borrowed from the response buffer.

1. Add `#[serde(borrow)]` to `KalshiAccountResponse` struct fields where the values are JSON strings.
2. Add `Vec::with_capacity(expected_count)` before constructing Vecs from response data.
3. Apply `sonic_rs::from_slice` (from agent-07) for zero-copy parsing if the dependency is available.

**Fix #42 — Venue-router accept pre-constructed key:**

`crates/venue-router/src/lifecycle.rs` (around lines 78–80) — function accepts `&str` parameters and calls `.to_owned()` internally to build a key.

1. Change the function to accept `CollectorKey` directly (the typed struct added in agent-10):
   ```rust
   // Before:
   pub fn start_collector(&self, lane: &str, instrument: &str, venue: &str)
   // After:
   pub fn start_collector(&self, key: CollectorKey)
   ```
2. The caller, who already has `Arc<str>` values, constructs the `CollectorKey` once. No `.to_owned()` inside the function.

**Fix #41 — Pre-normalize strings in reconciliation:**

`crates/reconciliation/src/positions.rs` (around lines 35–36) — `.replace("-", "_")` or similar normalization called per-comparison in the reconciliation loop, allocating a new String each time.

1. Move the normalization to data-load time: when positions are first deserialized from the broker API response, normalize the symbol strings once.
2. At comparison time, both sides are already normalized — no per-comparison allocation.

**Fix #50 — Single-pass lanes iteration:**

`crates/event-bus/src/nats.rs` (around lines 51–81) — two passes over a 12-item lanes array (subscribe to subjects, then map subjects to handlers).

1. Combine into one pass:
   ```rust
   let subscriptions: Vec<(Subject, Handler)> = LANES.iter()
       .map(|lane| {
           let subject = make_subject(lane);
           let handler = make_handler(lane);
           (subject, handler)
       })
       .collect();
   ```
2. This is a negligible performance fix (12 items) but eliminates the conceptual debt of the split loop.

**Acceptance test for the group:**
- `cargo test` must pass for all affected crates after all changes.
- `grep -rn "serde_json::to_string" crates/api/src/ws/live.rs` returns zero (binary WS encoding in place).
- `grep -n "compile_manifest" crates/strategy-runtime/src/manifest.rs` shows no HashSet construction inside the function.
- `AssetClass::as_str()` method exists and returns `&'static str`.

## Overall Acceptance Criteria
- [ ] WS messages use binary postcard encoding, not `serde_json::to_string` per frame
- [ ] `panel_id` and `instrument_id` passed by reference in WS dispatch, not cloned
- [ ] Rollup grouping is single-pass with `HashMap::entry()`
- [ ] Error construction uses typed `thiserror` enums — no `format!(...)` at error-construction time
- [ ] `AssetClass::as_str()` method exists; `serde_json::to_value` removed from graph populate
- [ ] `compile_manifest()` contains no HashSet construction or definition tree walk
- [ ] Feature names in StrategyDefinition use `Arc<str>`, not `String`
- [ ] `SOCIAL_COLLECTION.to_owned()` removed in `crates/semantic/src/lib.rs`
- [ ] Reddit title uses `as_deref()`, not `clone()`
- [ ] `cargo test` passes for all affected crates

## Files to Touch
- `crates/api/src/ws/live.rs` — postcard binary encoding; panel_id/instrument_id by reference
- `crates/api/src/rollup/mod.rs` — single-pass grouping with entry()
- `crates/api/src/routes/dashboard.rs` — thiserror typed errors; no format!() at construction
- `crates/api/src/routes/strategies.rs` — AssetClass/OrderStatus as_str(); no format!("{:?}")
- `crates/api/src/routes/venue_health.rs` — thiserror typed errors
- `crates/strategy-runtime/src/manifest.rs` — dedup at definition load; no HashSet in compile_manifest()
- `crates/strategy-runtime/src/runtime.rs` — Arc<str> for instrument_id HashMap keys
- `crates/strategy-runtime/tests/manifest_compile.rs` — remove unnecessary clone
- `crates/graph/src/populate.rs` — AssetClass::as_str(); collect &'static str not String
- `crates/graph/src/schema.rs` — CSR flat layout instead of Vec<Vec<T>>
- `crates/semantic/src/lib.rs` — remove SOCIAL_COLLECTION.to_owned()
- `crates/collectors/src/social/reddit.rs` — title as_deref; Arc<HashSet> for known_instruments
- `crates/execution/src/account/kalshi.rs` — serde(borrow); Vec::with_capacity
- `crates/venue-router/src/lifecycle.rs` — accept CollectorKey directly; remove .to_owned()
- `crates/reconciliation/src/positions.rs` — normalize at load time, not per comparison
- `crates/event-bus/src/nats.rs` — single-pass lanes iteration
- `Cargo.toml` — add `postcard = "1"`, `thiserror = "1"` to workspace dependencies
