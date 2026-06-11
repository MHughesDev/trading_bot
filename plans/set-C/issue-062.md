# Issue #062 — Milvus: .to_owned() on static strings

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | 2 clones per collection init |
| Location | `crates/semantic/src/lib.rs:90` |

## Problem
`SOCIAL_COLLECTION.to_owned()` and `EMBEDDING_MODEL.to_owned()` clone static `&str` constants on every call to the collection init function. These are fixed strings that should be passed as `&'static str` or wrapped in `Arc<str>` once at startup.

## Root Cause
`SOCIAL_COLLECTION` and `EMBEDDING_MODEL` are likely defined as `const &str` or `static str`. Calling `.to_owned()` on them allocates a heap String on every use — unnecessary since the strings never change.

## Implementation Plan
### Step 1 — Change the Milvus client to accept &str instead of String
If the Milvus client function at `lib.rs:90` accepts a `String`, check if the client API can accept `&str` or `Cow<str>` instead. If the client takes a `String`, file this as a client library limitation.

### Step 2 — Define constants as lazy_static or once_cell Arc<str>
```rust
static SOCIAL_COLLECTION_ARC: once_cell::sync::Lazy<Arc<str>> =
    once_cell::sync::Lazy::new(|| Arc::from("social_embeddings"));
```
Clone the Arc (atomic increment) on each use.

### Step 3 — Alternative: accept &str in the Milvus wrapper
Wrap the Milvus client in a local abstraction that accepts `&str` and converts to `String` once per request, not per call-site:
```rust
fn init_collection(&self, name: &str) {
    self.client.create_collection(name.to_string(), ...);
}
```
Collection init is infrequent (startup only) — even a String allocation here is acceptable.

### Step 4 — Document frequency of collection init
If collection init is called only at startup, this is truly negligible. Document that the fix is for hygiene / pattern consistency, not performance.

## Acceptance Criteria
- [ ] No `.to_owned()` on static string constants at `lib.rs:90`
- [ ] `SOCIAL_COLLECTION` and `EMBEDDING_MODEL` not heap-allocated on every call
- [ ] Milvus integration test passes: collection correctly initialized

## Files to Change
- `crates/semantic/src/lib.rs` — remove .to_owned() on static strings at line 90; use Arc<str> or &str interface
