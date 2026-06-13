# Phase 1 — Authentication & Multi-User

**Completion: 0% (0 / 7 tasks complete)**

**Goal:** Replace the M-17 placeholder auth with real session authentication,
scope every resource to its owner, and only then unlock network binding.
**Addresses:** #1, #2, #3, #4, #5, #6. **Resolves CRITICAL item M-17.**

> **Keystone finding:** the frontend already ships a complete cookie-session
> auth UX (`frontend/src/lib/api.ts` `authApi`, `store/auth.ts`, `LoginPage`,
> `SignUpPage`) calling `/auth/login|register|me|logout|touch` and
> `/auth/venue-credentials*`. **None of these routes exist in the Rust API**
> (`routes/mod.rs` registers none; no `argon2`/`jsonwebtoken`/session crate in
> any `Cargo.toml`). Building that route module (1.2) is the missing backend the
> other items depend on.
>
> **Open decisions 1–3 in MASTER must be resolved before starting.**

---

## Tasks

### ☐ 1.1 User + session store (migration + repository) — M
**Addresses #4 (CL users.rs).** `storage/postgres/users.rs` has only `count()`;
the `users` table (`migrations/0001`) has **no password column**.
- Migration `0013_user_auth.sql`: `ALTER TABLE users ADD COLUMN password_hash
  TEXT`; new `sessions` table (`session_id PK`, `user_id FK`, `token_hash BYTEA`
  = SHA-256 of a 256-bit random token, `created_at`, `expires_at`,
  `last_seen_at`).
- Add `argon2` (Argon2id) to the auth/storage crate. Implement `create_user`,
  `find_by_email` (returns hash for verify), `get_by_id` (for `/auth/me`),
  plus session create/lookup/touch/revoke.
- **Files:** `migrations/0013_user_auth.sql`, `crates/storage/src/postgres/users.rs`,
  new `sessions` module, `crates/storage/Cargo.toml`.
- **Verify:** round-trip tests for user create/verify (Argon2id) and session
  create/lookup/expire; never log hashes.

### ☐ 1.2 `/auth/*` route module — M — **keystone**
**Addresses #3 (NF).** Build the backend the frontend already calls.
- `register`, `login` (mint a session, set cookie), `me`, `logout` (revoke),
  `touch` (sliding expiry); wire the existing `credentials/crypto.rs`
  venue-credentials endpoints. Register all in `routes/mod.rs`.
- Cookie shape per Open Decision 1 (default: opaque session cookie,
  `SameSite=Lax/Strict`, `HttpOnly`, `Secure`).
- **Files:** new `crates/api/src/routes/auth.rs`, `routes/mod.rs:20-85`,
  `crates/api/src/credentials/`.
- **Verify:** integration test of register → login → me → logout against a test
  Postgres; constant-time failure on bad credentials.

### ☐ 1.3 Real session-validating extractor — M
**Addresses #1 (CL session.rs CRITICAL).** `BearerToken` accepts any non-empty
token and derives a `UUIDv5` identity (`session.rs:28-30,42-62`).
- Replace with a `CurrentUser(Uuid)` extractor that reads the session token
  from cookie **or** `Authorization: Bearer`, hashes it, looks it up with
  constant-time comparison, checks `expires_at`, and yields a real
  `users.user_id`. Switch to `FromRequestParts<AppState>` (needs state).
- Keep a Bearer path so existing route signatures keep working; remove the
  UUIDv5-of-token trick.
- **Files:** `crates/api/src/auth/session.rs`, every route's extractor bound.
- **Verify:** invalid/expired/missing → 401; valid → correct `user_id`.

### ☐ 1.4 Scope automations + unify `streams.rs` identity — S–M
**Addresses #5 (CL auth + NF).** `automations.rs` ignores the token and writes
`DEV_USER = Uuid::nil()` (`:27,:102`); storage queries (`storage/automation.rs`)
have **no user filter** — any token can list/arm/disarm/delete every user's
automations. `streams.rs:48` keys the gateway on the **raw token string** while
backtests use `token.user_id()`.
- Mirror the backtests scoping exactly: `create` stamps `user_id`; `list`
  filters `WHERE user_id=$1`; arm/disarm/delete take `(user_id, id)` and 404 on
  non-ownership. The `automations.user_id` column already exists (no schema
  change).
- Unify `streams.rs:48` onto the real `user_id` (confirm Open Decision: not an
  intentional gateway keying scheme).
- **Files:** `crates/api/src/routes/automations.rs`,
  `crates/storage/src/automation.rs`, `crates/api/src/routes/streams.rs:48`.
- **Verify:** cross-user list/arm/delete returns 404; create stamps the caller.

### ☐ 1.5 Drop the frontend `dev-local` interceptor — S
**Addresses #6 (NF).** The SPA injects `Authorization: Bearer dev-local` on
every request (`api.ts:11-16`). With real cookie sessions (`withCredentials`
already set) this is no longer needed.
- Delete the request interceptor; keep `withCredentials: true`. `authApi`,
  `useAuthStore`, `LoginPage`, `SignUpPage` are already correct for a cookie
  backend. (If Open Decision 1 picks JWT instead, this becomes a token-storage
  rewrite — confirm first.)
- **Files:** `frontend/src/lib/api.ts:11-16`.
- **Verify:** authenticated requests succeed via cookie alone; 401 redirects to
  `/login` (interceptor at `api.ts:18-31` already does this).

### ☐ 1.6 Legacy nil-user data migration — S
**Addresses Open Decision 3.** Existing automations/backtests under
`Uuid::nil()` and the old UUIDv5-of-token identity orphan once real `user_id`s
land. **Armed live automations under nil move real money** — they must not
silently detach.
- Migration (or one-off admin task) to reassign or quarantine nil-user rows to
  a seeded operator account; surface armed-automation reassignment explicitly.
- **Files:** `migrations/0014_legacy_user_reassignment.sql` (or admin tool).
- **Verify:** no armed automation is left owner-less after migration.

### ☐ 1.7 Flip the loopback bind guard — S — **security gate, LAST**
**Addresses #2 (CL main.rs guardrail).** `main.rs:169-180` refuses to bind
non-loopback while auth is placeholder.
- Do **not** just delete it. Flip the predicate from "is loopback" to "is auth
  hardened": refuse to bind a non-loopback interface unless real session auth is
  active and no dev bypass is enabled. Add TLS / reverse-proxy + CSRF guidance
  (cookies now cross a network).
- **Merge this last**, after 1.1–1.6 are verified end-to-end, and run
  `/security-review` on the diff before exposing.
- **Files:** `apps/platform/src/main.rs:169-180`, config.
- **Verify:** platform refuses non-loopback bind when auth is not hardened;
  binds successfully when it is; security review clean.

---

## Definition of Done
A user can register/log in/out through the existing UI against the real Rust
backend; every authenticated route resolves a real `user_id`; automations and
streams are owner-scoped like backtests; the `dev-local` token is gone; legacy
nil-user rows are reassigned; and the platform can bind a network interface only
when auth is hardened, with a clean security review.
