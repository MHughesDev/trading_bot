# Phase 1 — Authentication & Multi-User

**Completion: 6 / 7 DONE (1.1–1.6) · 1.7 PARTIAL (guard wired; the `/security-review` is a human gate) — track PROD, 2026-10-03**

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
> **Locked decisions (2026-06-13):** (1) opaque server-side **cookie sessions**;
> (2) **multi-tenant with hard cross-user isolation**; (3) legacy `Uuid::nil()`
> rows are **deleted** on cutover (armed automations disarmed first). Add CSRF
> protection (cookie auth). See MASTER → Locked decisions.

---

## Tasks

### ☑ 1.1 [DONE 2026-10-03] — User + session store (migration + repository) — M

> **2026-10-03 (track PROD).** Migration **0150**: `sessions.token` now holds
> `session_token_hash(token)` (hex SHA-256, one SQL definition); a BEFORE INSERT
> trigger hashes whatever is inserted, so no writer can store cleartext; the key
> can never be rewritten (UPDATE refused); `CHECK (token ~ '^[0-9a-f]{64}$')`;
> live rows re-hashed in place, expired ones dropped. `last_seen_at` added.
> Sliding expiry in `auth::session::resolve`: web sessions slide to
> `now()+7d`, capped at `created_at+30d`, throttled to one write a minute;
> service sessions never slide. Expired sessions are swept lazily from the
> extractor (`maybe_sweep`, once per 10 min per process) — reached by every
> authenticated request, no spawn needed. Also fixed: web logins were minted
> with scopes `{}` (column default) since 0038, so a logged-in human held no
> scope (the kill switch refused them); login/register now insert `web:full` and
> 0150 repairs existing rows.
> **ADR-E-1.1** (deviation): the column keeps the name `token` (TEXT, hex) rather
> than becoming `token_hash BYTEA`. Rejected alternative: a rename, which
> silently breaks every `WHERE token = $1` reader outside auth (strategy
> authorship, research attribution, agent audit trail). `BearerToken::token` now
> carries the digest (the session key), so those readers stay correct unedited.
> **Evidence:** scratch DB (PG17 :5433) — 0150 applied over 0001–0149, the hashed
> row matched `session_token_hash('cleartext-old')`, an insert as `app_role`
> stored the digest, an UPDATE of the key was refused; `the_websocket_sockets_
> verify_their_token` (source assert now pins the hashed lookup).
> **Three readers outside this track still bind a *cleartext* token** and need
> the one-word edit listed in the track report: `llm_proxy.rs:170` (lookup),
> `orchestrator.rs:116` (revoke), `agent/manager.rs:655,685` (run-token delete).
> Until applied, the LLM proxy 401s for agent containers and revocation of those
> tokens waits for expiry.
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

### ☑ 1.2 [DONE 2026-10-03] `/auth/*` route module — M — **keystone**

> **2026-10-03 (track PROD).** `POST /auth/touch` (`auth::handlers::touch`,
> slides a web session, reports `extended:false` for a service session) and
> `GET|PUT /auth/venue-credentials`, `DELETE /auth/venue-credentials/{venue}`,
> `POST /auth/venue-credentials/{venue}/verify` (`auth::venue_credentials`). Credentials
> are one AES-256-GCM envelope per venue (`credentials::CredentialCrypto`) in
> `venue_credentials` — 0150 adds the `nonce`/`wrapped_dek`/`envelope_key_version`
> columns the 0007 table lacked, plus `field_names`/`key_last4`. No route
> decrypts; no KEK → 503, nothing stored. `verify` is an honest **501**: verifying
> means calling the venue, which is E-3.x. Both SPA body shapes are accepted
> (`{venue, …fields}` from Settings and venue-prefixed fields from SignUp).
> Tests: `auth::venue_credentials::tests` (4) and, against Postgres,
> `pg_sessions_and_ownership` (encrypted at rest: the ciphertext column holds no
> plaintext; owner-scoped read/delete; no KEK stores nothing; `app_role` holds
> the grants). Mounted in `routes/mod.rs` (`/auth/touch`,
> `/auth/venue-credentials`, `/{venue}`, `/{venue}/verify`) by the orchestrator, wave 1.
**Addresses #3 (NF).** Build the backend the frontend already calls.
- `register`, `login` (mint a session, set cookie), `me`, `logout` (revoke),
  `touch` (sliding expiry); wire the existing `credentials/crypto.rs`
  venue-credentials endpoints. Register all in `routes/mod.rs`.
- Cookie shape (locked decision 1): opaque session cookie,
  `SameSite=Lax/Strict`, `HttpOnly`, `Secure`.
- **Files:** new `crates/api/src/routes/auth.rs`, `routes/mod.rs:20-85`,
  `crates/api/src/credentials/`.
- **Verify:** integration test of register → login → me → logout against a test
  Postgres; constant-time failure on bad credentials.

### ☑ 1.3 [DONE — verified 2026-10-03] Real session-validating extractor — M

> **2026-10-03 (track PROD), verify-and-close.** The 2026-09-22 fix holds: the
> only two WebSocket upgrades in the crate (`ws/live.rs::ws_live`,
> `ws/backtest.rs::ws_backtest_suite`) take `WsBearerToken`, which now calls the
> one resolver `auth::session::resolve` (hashed lookup + expiry + sliding
> expiry) instead of its own query. REST and WS cannot disagree about a token.
> Evidence: `the_websocket_sockets_verify_their_token` (source + live halves).
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

### ☑ 1.4 [DONE 2026-10-03] Scope automations + unify `streams.rs` identity — S–M

> **2026-10-03 (track PROD).** `DEV_USER = Uuid::nil()` is gone. `create`
> stamps `token.user_id()`; `list` is `list_automations_for(user)`; arm, disarm
> and delete check `get_automation_owned` / `disarm_automation_owned` /
> `delete_automation_owned` (`WHERE id = $1 AND user_id = $2`) and answer 404 for
> a stranger's id exactly as for a missing one. The unscoped `list_automations`
> and `delete_automation` were deleted so nothing can reach for them. Deleting an
> armed automation now also deregisters it from the live registry (it used to
> keep evaluating with no row). New `arm_owned(state, owner, id)`.
> **Closed (wave 2):** `routes/proposals.rs` `deploy_paper` now arms through
> `arm_owned(state, <approving user>, subject)`; a subject owned by anyone else
> reads as "no such automation". `arm_by_id` is unscoped by design and is called
> only by `arm_owned`.
**Addresses #5 (CL auth + NF).** `automations.rs` ignores the token and writes
`DEV_USER = Uuid::nil()` (`:27,:102`); storage queries (`storage/automation.rs`)
have **no user filter** — any token can list/arm/disarm/delete every user's
automations. `streams.rs:48` keys the gateway on the **raw token string** while
backtests use `token.user_id()`.
- Mirror the backtests scoping exactly: `create` stamps `user_id`; `list`
  filters `WHERE user_id=$1`; arm/disarm/delete take `(user_id, id)` and 404 on
  non-ownership. The `automations.user_id` column already exists (no schema
  change).
- Unify `streams.rs:48` onto the real `user_id` (it currently keys the gateway
  on the raw token string — an inconsistency to fix, not an intentional scheme).
- **Files:** `crates/api/src/routes/automations.rs`,
  `crates/storage/src/automation.rs`, `crates/api/src/routes/streams.rs:48`.
- **Verify:** cross-user list/arm/delete returns 404; create stamps the caller.

### ☑ 1.5 [DONE — verified 2026-10-03] Drop the frontend `dev-local` interceptor — S

> **2026-10-03 (track PROD), verify-and-close.** No code path sends or opens a
> socket with `dev-local`: every remaining occurrence in `frontend/src` is a
> comment or a test. `lib/api.ts` sends `Authorization` only when a token is
> stored; `AppLayout.tsx`, `useConnection`, `useSuiteProgress` open no socket
> without one. Evidence: `an-absent-token-is-not-a-credential.test.ts` (9) and
> `a-signed-out-session-is-not-a-failed-connection.test.tsx` (6) pass. Auth
> stays bearer-in-`localStorage` (see 1.7 on why that means no CSRF surface).
**Addresses #6 (NF).** The SPA injects `Authorization: Bearer dev-local` on
every request (`api.ts:11-16`). With real cookie sessions (`withCredentials`
already set) this is no longer needed.
- Delete the request interceptor; keep `withCredentials: true`. `authApi`,
  `useAuthStore`, `LoginPage`, `SignUpPage` are already correct for a cookie
  backend. (Locked decision 1 is cookie sessions, so this is a clean deletion,
  not a token-storage rewrite.)
- **Files:** `frontend/src/lib/api.ts:11-16`.
- **Verify:** authenticated requests succeed via cookie alone; 401 redirects to
  `/login` (interceptor at `api.ts:18-31` already does this).

### ☑ 1.6 [DONE 2026-10-03] Delete legacy nil-user data — S

> **2026-10-03 (track PROD).** Migration **0151**: disarm, then delete, every
> automation owned by nil *or* by no `users` row (the UUIDv5-of-token identity);
> delete nil/NULL-owner backtests; then `CHECK (user_id <> nil)` so none can be
> written again. Orphaned non-nil backtests are kept (inert history — deleting
> research results is a bigger decision than this cutover). Evidence: scratch DB
> scenario — an armed nil row and an armed orphan row were both gone, a real
> user's row survived, and a later nil insert was refused by the CHECK.
**Locked decision 3: delete.** Existing automations/backtests under
`Uuid::nil()` and the old UUIDv5-of-token identity orphan once real `user_id`s
land. The cutover **deletes** them — but **armed live automations under nil move
real money**, so they must be **disarmed first** as a safety step, then removed.
- Migration: first set `armed = false` for any nil-user automation, then
  `DELETE FROM automations WHERE user_id = '00000000-...'` and the matching
  `backtest_runs` nil-user rows. Order matters: disarm before delete so nothing
  fires during the migration.
- **Files:** `migrations/0014_drop_legacy_nil_user_data.sql`.
- **Verify:** post-migration there are zero nil-user automations/backtests and no
  automation was left armed mid-migration.

### ◑ 1.7 [PARTIAL — guard wired in `main.rs`; clean `/security-review` outstanding (human gate)] Flip the loopback bind guard — S — **security gate, LAST**

> **2026-10-03 (track PROD).** `api::auth::hardening::require_bindable(host, pg)`:
> loopback always binds; a network bind requires (1) 0150's hash trigger and
> CHECK present, (2) 0151's owner CHECK present, (3) no dev-bypass env var set,
> (4) `TRADING_TLS_TERMINATED=1` declaring a TLS proxy. Database facts fail
> closed. CSRF: auth is a bearer header from `localStorage`, never a cookie, so a
> cross-site request carries no credential; the guard's doc says a move to
> cookies must add a CSRF check first. Tests: `auth::hardening::tests` (2).
> **Wired** in `apps/platform/src/main.rs` (`require_bindable` before the bind),
> now that 1.1–1.6 all hold. **Remaining:** the acceptance clause "security review
> clean" — run `/security-review` on the diff before any operator sets
> `TRADING_TLS_TERMINATED=1` and binds a network interface. Until a human does,
> this is not marked DONE.
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
