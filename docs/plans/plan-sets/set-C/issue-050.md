# Issue #050 — Multiple iterations over lanes collection

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Algorithm |
| Quick Win | Yes |
| Latency Impact | 2 passes over 12-item array (startup only) |
| Location | `crates/event-bus/src/nats.rs:51-81` |

## Problem
The lanes array is iterated twice at startup. This is a startup-only cost, not a hot-path issue, but represents a negligible code hygiene improvement and serves as documentation of clean-up opportunities in the event-bus crate.

## Root Cause
The NATS setup code at lines 51-81 performs two passes over the lanes array (likely: one to create streams, one to create consumers or bindings). These could be combined into a single pass.

## Implementation Plan
### Step 1 — Read nats.rs:51-81 to understand both passes
Identify what each pass does:
- Pass 1: likely creates JetStream streams for each lane
- Pass 2: likely creates consumers or configures subscriptions

### Step 2 — Combine into a single pass
```rust
for lane in &lanes {
    create_stream(lane).await?;
    create_consumer(lane).await?;
}
```
Instead of two separate for loops. The operations may have ordering requirements; if stream must exist before consumer, the single-pass approach still satisfies this.

### Step 3 — Verify startup correctness
Run the event-bus integration test to confirm that streams and consumers are correctly created with the single-pass approach.

## Note on Priority
This is a startup-only, negligible-cost issue. It should be the last item addressed in Phase E — only if all higher-priority issues are resolved and the team has bandwidth.

## Acceptance Criteria
- [ ] Single iteration over lanes at startup in `nats.rs:51-81`
- [ ] All streams and consumers created correctly (integration test passes)
- [ ] No startup behavior regression

## Files to Change
- `crates/event-bus/src/nats.rs` — combine two-pass lane iteration into single pass at lines 51-81
