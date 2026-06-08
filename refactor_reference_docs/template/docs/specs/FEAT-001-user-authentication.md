# Example Spec — Delete This File After Reading

> This file demonstrates the spec format end-to-end, including: the naming convention
> (FEAT-001-…), all seven numbered sections with N.M.X addressing, a requirements section
> with both functional and non-functional items, a user stories block, behavior covering the
> happy path + multiple edge cases + error states, acceptance criteria that cite their
> requirements, and a split note showing when this spec would split into two.

---

# Spec: FEAT-001 — User Authentication

**Spec ID:** FEAT-001
**Type:** Feature
**Status:** Approved
**Date:** 2024-03-10
**Author:** Agent

## 1. Overview

1.1 Purpose — Defines how users sign up, log in, and access their workspace. Covers all
   user-facing authentication flows: email/password registration, Google OAuth, email
   verification, session persistence, and workspace access enforcement.

1.2 Context — This is the entry point of the application; every other feature sits behind it.
   Authentication is handled by Clerk (see ADR-0001); this spec defines what the app owns —
   the flows, error states, and routing behavior — not what Clerk owns internally (token
   issuance, session storage, revocation). The workspace data model is specified separately in
   `DATA-001-workspace-schema.md` (to be created); this spec references workspace creation as
   a step but does not define the schema.

   > **Split note:** If workspace creation logic grows beyond "create default on first login"
   > (e.g., workspace naming, plan selection, team invites at sign-up), that block should be
   > extracted into its own `FEAT-002-workspace-onboarding.md`. The split boundary is clear
   > because workspace creation can be implemented and tested independently of the auth flow.

1.3 Related artifacts
   1.3.A ADR: [adr/0001-example-use-clerk-for-auth.md](../adr/0001-example-use-clerk-for-auth.md)
   1.3.B Research: [research/example-auth-provider-comparison.md](../research/example-auth-provider-comparison.md)
   1.3.C Open questions: [open-questions.md](../open-questions.md) — Q-2, Q-3
   1.3.D Plan: [plans/example-task-management-app.md](../plans/example-task-management-app.md)

## 2. Scope

2.1 Goals
   2.1.A A new user can create an account and reach their workspace in under 2 minutes.
   2.1.B Email/password and Google OAuth are both supported at launch.
   2.1.C A user can only access workspaces they are a member of — no cross-workspace data leakage.
   2.1.D Repeated login failures are rate-limited to deter brute-force attacks.
   2.1.E Session state persists across browser refreshes without re-authentication.

2.2 Non-Goals (out of scope)
   2.2.A SSO / SAML enterprise login — deferred post-launch; no stub or placeholder code.
   2.2.B Passwordless / magic-link login — deferred post-launch.
   2.2.C Session management internals (token issuance, rotation, revocation) — owned by Clerk.
   2.2.D Account deletion and data export flows — separate spec required when prioritized.
   2.2.E Multi-device session management (viewing or revoking active sessions) — separate spec.

## 3. Requirements

3.1 Functional requirements
   3.1.A A user can register with an email address and password.
   3.1.B A user can register and log in with a Google account (OAuth 2.0).
   3.1.C Email registration requires email address verification before access is granted.
   3.1.D A successfully verified new user is automatically placed into a newly created default
         workspace named after their email domain (or "My Workspace" if the domain is a public
         provider such as gmail.com).
   3.1.E A logged-in user is redirected to their workspace dashboard after every successful login.
   3.1.F A user can only access routes scoped to a workspace they are a member of.
   3.1.G A logged-out user attempting to access a protected route is redirected to `/login`
         with the original URL preserved as a `?redirect` query param.

3.2 Non-functional requirements
   3.2.A After 5 consecutive failed login attempts from the same account within 10 minutes,
         the account is locked for 30 seconds before another attempt is permitted.
   3.2.B The sign-up flow (from landing on `/sign-up` to reaching the workspace dashboard)
         must be completable in under 2 minutes on a standard broadband connection.
   3.2.C All auth-related redirects must complete within 500ms (P95) excluding Clerk API
         round-trip time.
   3.2.D Error messages must not expose internal error codes, stack traces, or Clerk API
         error details to the user.

## 4. Interface / Data

4.1 User stories

   4.1.A **Sign up with email**
         As a new user, I want to create an account with my email and a password I choose,
         so that I have a private, persistent identity in the app.

   4.1.B **Sign up with Google**
         As a new user, I want to register using my Google account without filling in a form,
         so that I can start using the app in as few steps as possible.

   4.1.C **Log in**
         As a returning user, I want to log in with my email/password or Google account,
         so that I can resume my work in my workspace.

   4.1.D **Access my workspace**
         As a logged-in user, I want to land directly on my workspace dashboard after login,
         so that I can see my tasks without additional navigation.

   4.1.E **Be protected from accidental cross-workspace access**
         As a user, I want to be confident that I cannot accidentally view or modify another
         customer's workspace, even if I know or guess a workspace URL.

## 5. Behavior

5.1 Happy path — email sign-up
   5.1.A User navigates to `/sign-up`.
   5.1.B User enters a valid email address and a password (minimum 8 characters).
   5.1.C User submits the form; Clerk creates the account and sends a verification email.
   5.1.D User clicks the verification link in their email.
   5.1.E App receives the verified session from Clerk, creates a default workspace (§3.1.D),
         and redirects the user to `/dashboard`.

5.2 Happy path — Google OAuth
   5.2.A User navigates to `/sign-up` or `/login` and clicks "Continue with Google".
   5.2.B Google OAuth consent screen is shown; user grants permission.
   5.2.C Clerk receives the OAuth callback, creates or retrieves the user account.
   5.2.D If first login: app creates a default workspace (§3.1.D) and redirects to `/dashboard`.
   5.2.E If returning user: app redirects directly to `/dashboard` with existing workspace.

5.3 Happy path — login with redirect preservation
   5.3.A A logged-out user visits `/workspaces/abc123/tasks`.
   5.3.B App detects no session, redirects to `/login?redirect=/workspaces/abc123/tasks`.
   5.3.C User logs in successfully.
   5.3.D App reads the `redirect` param and sends the user to `/workspaces/abc123/tasks`.
   5.3.E If the redirect target is a workspace the user is not a member of, ignore the redirect
         and send the user to their own `/dashboard` instead.

5.4 Edge cases
   5.4.A **Email already registered (email/password path):**
         Show inline form error: "An account with this email already exists. Log in instead."
         The error links to `/login`. Do not send a verification email or create a duplicate
         account. Show error within 1 second of submission (§3.2.C).

   5.4.B **Google account previously linked to an email/password account:**
         Clerk handles account merging via its account linking flow. The app does not
         implement custom merge logic — it accepts the merged session from Clerk and proceeds.

   5.4.C **Verification email not received:**
         After the verification-pending screen has been shown for 60 seconds without the user
         confirming, display a "Resend verification email" button. Clicking it triggers a new
         verification email via Clerk. Show a "Email resent" confirmation inline.

   5.4.D **User attempts to access a workspace they are not a member of:**
         Return HTTP 403. Do not redirect to login (the user is authenticated — the issue is
         authorization). Show an in-app error page: "You don't have access to this workspace."
         Provide a button linking to the user's own `/dashboard`.

   5.4.E **User's account has no workspace (e.g., workspace was deleted):**
         Redirect to `/onboarding/create-workspace` rather than `/dashboard`. This state
         should be rare but must be handled gracefully — do not show a blank dashboard or 500.

   5.4.F **`redirect` param in login URL points to an external domain:**
         Ignore the redirect param entirely and send the user to `/dashboard`. Never redirect
         to an external URL from an auth callback — open redirect vulnerability (§3.2.D).

5.5 Error states
   5.5.A **Clerk API unavailable (5xx from Clerk):**
         Show: "Sign-in is temporarily unavailable. Please try again in a moment."
         Log the Clerk error server-side. Do not surface the Clerk status code or error message
         to the user (§3.2.D). Retry logic is Clerk's SDK responsibility, not the app's.

   5.5.B **Invalid email format submitted:**
         Show inline validation: "Enter a valid email address." before form submission
         (client-side). Do not call Clerk for an invalid email format.

   5.5.C **Password too short:**
         Show inline validation: "Password must be at least 8 characters." before submission.

   5.5.D **Login rate limit reached (§3.2.A):**
         Show: "Too many attempts. Please wait 30 seconds before trying again." Do not expose
         the account lockout status to unauthenticated callers in a way that enables account
         enumeration (same message whether or not the email exists).

   5.5.E **Google OAuth consent denied by user:**
         Return user to `/sign-up` with no error message shown (the user deliberately declined;
         no failure has occurred). Log the abandonment event for analytics.

   5.5.F **Verification link expired (Clerk default: 24 hours):**
         Show: "This verification link has expired. Request a new one." with a button to
         re-trigger verification from `/sign-up` pre-filled with the original email.

## 6. Acceptance Criteria

6.1 Criteria

   6.1.A [x] A new user completes email sign-up (form → verification email → click link →
             workspace dashboard) in under 2 minutes on a standard broadband connection.
             (verifies §3.1.A, §3.1.C, §3.1.D, §3.2.B) — Verified by: e2e/signup.spec.ts, 2024-04-02

   6.1.B [ ] A new user signs up via Google OAuth and reaches their workspace dashboard
             without filling in any form fields.
             (verifies §3.1.B, §3.1.D) — Verified by: [—]

   6.1.C [x] A logged-in user visiting `/workspaces/:id/tasks` where `:id` is a workspace
             they are NOT a member of receives a 403 response and sees the "no access" page.
             (verifies §3.1.F) — Verified by: e2e/workspace-isolation.spec.ts, 2024-04-03

   6.1.D [ ] A logged-out user visiting a protected route is redirected to
             `/login?redirect=<original-path>` and, after login, is sent to the original path.
             (verifies §3.1.G) — Verified by: [—]

   6.1.E [ ] After 5 consecutive failed login attempts on the same account within 10 minutes,
             a 6th attempt within 30 seconds is rejected with the rate-limit error message.
             (verifies §3.2.A) — Verified by: [—]

   6.1.F [ ] The "email already registered" inline error appears within 1 second of submission.
             (verifies §5.4.A, §3.2.C) — Verified by: [—]

   6.1.G [ ] A `?redirect` param pointing to an external domain (e.g., `?redirect=https://evil.com`)
             is ignored; the user lands on `/dashboard` after login.
             (verifies §5.4.F) — Verified by: [—]

   6.1.H [ ] When Clerk returns a 5xx, the user sees the generic "temporarily unavailable"
             message and no internal error details appear in the page or network response body.
             (verifies §5.5.A, §3.2.D) — Verified by: [—]

   6.1.I [ ] A user with no workspace (all workspaces deleted) who logs in is redirected to
             `/onboarding/create-workspace`, not `/dashboard`.
             (verifies §5.4.E) — Verified by: [—]

## 7. Open Questions & Assumptions

7.1 Open questions — see [open-questions.md](../open-questions.md)
   - Q-2: Does Clerk's SOC 2 Type II scope explicitly cover the Organizations feature?
   - Q-3: Is Clerk's EU data residency contractually lockable per-account, not just per-tenant?

7.2 Assumptions (unvalidated ones are uncertainty sources — see `trace-uncertainty.md`)
   7.2.A Clerk handles all session token issuance, rotation, and revocation. The app
         does not implement a parallel session layer. — Validated: ADR-0001 §Rationale, 2024-03-08
   7.2.B The app receives a verified Clerk session object and trusts it without re-validating
         the JWT signature on every request (Clerk middleware handles this). — Validated: [—]
   7.2.C "Default workspace" creation (§3.1.D) is synchronous with the first login redirect.
         If workspace creation fails, the user should see an error rather than a blank
         dashboard — error handling for this case is out of scope for this spec but must be
         addressed in `DATA-001`. — Validated: [—]
   7.2.D Email domain detection for workspace naming uses a simple split on `@` and a
         hardcoded list of public provider domains (gmail.com, yahoo.com, outlook.com, etc.)
         to determine whether to use the domain or fall back to "My Workspace". — Validated: [—]
