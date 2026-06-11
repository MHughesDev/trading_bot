# Issue #023 — Strategy manifest cloned per compile (test code)

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | 1 Vec clone (test code) |
| Location | `crates/strategy-runtime/tests/manifest_compile.rs:80` |

## Problem
The strategy manifest is cloned unnecessarily in test setup. While this only affects test execution time (not production performance), it's a code hygiene issue that also may reflect how the manifest is used in production code if the pattern is copied.

## Root Cause
Test setup at `manifest_compile.rs:80` calls `.clone()` on the manifest before passing it to the compile function. The compile function likely consumes the manifest, so the clone is needed to retain a copy for assertions — but the test could be restructured to avoid this.

## Implementation Plan
### Step 1 — Read manifest_compile.rs:80
Determine why the clone is performed. Options:
- (a) The manifest is needed both before and after compile — restructure test to verify before passing.
- (b) The compile function takes by value unnecessarily — change it to accept `&Manifest`.
- (c) The clone is part of testing clone behavior — accept and document.

### Step 2 — Change compile to accept &Manifest (preferred)
If the compile function takes `Manifest` by value and the caller needs to retain the original, change the signature to accept `&Manifest`. This eliminates the clone entirely.

### Step 3 — Restructure test assertions
Move all pre-compile assertions to before the compile call. Pass the manifest by reference or by move without cloning.

## Acceptance Criteria
- [ ] No `.clone()` call at `manifest_compile.rs:80` unless it is testing clone behavior specifically
- [ ] `compile_manifest` accepts `&Manifest` if it does not need ownership
- [ ] All manifest tests pass

## Files to Change
- `crates/strategy-runtime/tests/manifest_compile.rs` — remove clone at line 80; restructure test setup
