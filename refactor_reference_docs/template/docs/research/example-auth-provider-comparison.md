# Example Research Brief — Delete This File After Reading

> This file exists to demonstrate the research brief format end-to-end, including edge cases:
> a question that evolved mid-research, a comparison matrix with a non-obvious winner, open
> questions that surfaced new forks, and forward links to the ADR and spec it informed.

---

# Auth Provider Comparison: Auth0 vs. Clerk vs. Custom JWT

**Date:** 2024-03-01
**Researcher:** Agent
**Status:** Complete

## Question

Which authentication provider best fits a B2B SaaS app with multi-tenant workspace requirements,
a two-person engineering team, and a hard 6-month launch timeline?

> **Note:** This question evolved during research. The original question was "should we build or
> buy auth?" — but preliminary findings made build clearly unviable for a 2-person team, so the
> question was narrowed to comparing managed providers plus a custom option as a cost baseline.
> The scope section below reflects the final, narrower question.

## Scope

**In scope:**
- Auth0, Clerk, and a custom JWT/session implementation as a cost and control baseline
- Multi-tenancy support (workspace isolation per customer)
- Developer ergonomics with a Next.js + TypeScript stack
- Pricing at 10k, 50k, and 100k MAU (the expected range over the first 18 months)
- Compliance posture: SOC 2 Type II, GDPR data residency (EU customers expected from day one)
- Migration risk (what does moving away cost?)

**Out of scope:**
- SSO-only enterprise solutions (Okta, OneLogin, WorkOS) — out of budget for initial launch;
  deferred to a future research brief if enterprise sales materializes
- Passwordless-only approaches — email/password is a hard requirement per the artifact
- Mobile SDK evaluation — web-first launch only

## Method

- Read official documentation for Auth0 (accessed Feb 2024), Clerk (accessed Feb 2024)
- Read OWASP Authentication Cheat Sheet and JWT Security Cheat Sheet
- Reviewed Auth0 pricing calculator and Clerk pricing page; modeled 3 MAU scenarios
- Ran a 2-hour Clerk integration proof of concept against a Next.js 14 app router project
- Reviewed Auth0's Organizations feature docs and Clerk's Organizations docs
- Read 3 migration post-mortems on HN (auth0 → custom, custom → clerk, clerk → auth0) to
  understand migration cost in practice

## Findings

### Auth0

**Multi-tenancy:** Supported via Organizations. Available from the Pro plan ($240/month minimum
as of Feb 2024). The Organizations API is mature and well-documented. Tenant-level branding,
custom domains, and role assignment per org are all supported.

**Developer experience:** Integration is verbose. A standard Next.js integration requires
`@auth0/nextjs-auth0`, manual route protection, a custom session store hook, and significant
boilerplate around callback handling. The proof-of-concept was not run for Auth0, but public
integration guides suggest 5–8 days of integration work for a production-ready setup.

**Pricing (modeled):**
- 10k MAU: $240/month (Pro, minimum for multi-tenancy)
- 50k MAU: ~$850/month (Pro + MAU overage)
- 100k MAU: ~$1,500/month

**Compliance:** SOC 2 Type II certified. GDPR-compliant with EU data residency available on
Enterprise plans only — this is a blocker for EU customers on Pro.

**Migration risk:** Auth0 uses its own proprietary session and token format. Migrating out
requires coordinating token rotation across all active sessions and re-building the integration
layer. Estimated: 3–6 weeks for a team of 2.

**Specific finding — cold start latency:** Free and lower-tier plans show occasional cold start
latency (500ms–2s) on the auth API in regions outside US-East. This is not documented by Auth0
but appears consistently in community reports.

---

### Clerk

**Multi-tenancy:** Supported natively via Organizations. No plan gating — Organizations is
available on the free tier. Role-based access, invitations, and per-org metadata are built in.

**Developer experience:** Clerk provides a Next.js SDK (`@clerk/nextjs`) that wraps middleware,
session management, and UI components. The proof-of-concept reached a working sign-up + protected
route in ~2 hours. Session state is available via `useUser()` hook without custom plumbing.

**Pricing (modeled):**
- 10k MAU: $0 (free tier covers 10k MAU as of Feb 2024)
- 50k MAU: ~$175/month (Pro)
- 100k MAU: ~$350/month (Pro)

**Compliance:** SOC 2 Type II certified (confirmed Feb 2024). GDPR-compliant with EU data
residency on Pro and above — this satisfies the EU customer requirement. However, Clerk's data
residency is at the account level, not per-workspace — all workspaces share the same region
setting. This is a constraint if future customers require per-tenant regional isolation.

**Migration risk:** Clerk uses JWTs signed with RS256. Session tokens are standard enough that
migration to another provider primarily involves re-building the integration layer rather than
re-architecting session storage. Estimated: 2–3 weeks for a team of 2.

**Specific finding — SDK stability:** Clerk has made breaking changes between major SDK versions
(v3 → v4, v4 → v5 in quick succession). Each upgrade required non-trivial migration work per
community reports. The SDK is actively maintained but not yet stable in the way Auth0's is.

---

### Custom JWT Implementation

**Multi-tenancy:** Fully custom — can model any tenancy pattern, but must be built and maintained.

**Developer experience:** Requires implementing: token issuance (access + refresh), rotation on
refresh, revocation (token blocklist or short-lived tokens), secure storage guidance for clients,
PKCE for OAuth flows, and email verification. OWASP guidance identifies 12+ distinct implementation
attack surfaces for auth systems. Estimated implementation time: 3–5 weeks for a 2-person team
to reach a production-safe baseline.

**Pricing:** $0 recurring. Infrastructure cost is negligible (JWT signing is CPU-cheap).

**Compliance:** Compliance is entirely self-managed. SOC 2 audit scope expands to include the
custom auth system, which increases audit preparation cost and timeline.

**Migration risk:** Lowest vendor lock-in, but highest ongoing maintenance burden. Any security
vulnerability in the implementation is the team's to discover and patch.

---

## Comparison Matrix

| Criterion | Auth0 | Clerk | Custom JWT |
|-----------|-------|-------|------------|
| Multi-tenancy | Yes (paid only) | Yes (free tier) | Manual |
| Time to integrate | ~5–8 days | ~2 days (verified) | 3–5 weeks |
| EU data residency | Enterprise only | Pro+ (account-level) | Self-managed |
| Cost at 10k MAU | $240/month | $0 | $0 + dev time |
| Cost at 50k MAU | ~$850/month | ~$175/month | $0 + maintenance |
| Cost at 100k MAU | ~$1,500/month | ~$350/month | $0 + maintenance |
| SDK stability | High (mature) | Medium (active, breaking changes) | N/A |
| Migration cost out | High (3–6 wk) | Medium (2–3 wk) | Low (no vendor) |
| SOC 2 Type II | Yes | Yes | Must self-certify |
| Per-tenant regional isolation | Yes (Enterprise) | No | Yes (self-managed) |

## Recommendation

Use **Clerk** for the initial launch.

The 2-hour proof of concept confirms the 2-day integration estimate is realistic for a Next.js
stack. Multi-tenancy is available on the free tier (eliminating the $240/month minimum blocker
that rules out Auth0 at launch). EU data residency is available on Pro, satisfying the GDPR
requirement for the launch-day EU customer cohort.

The recommendation carries two known conditions under which a different choice would be better:
1. **If any customer requires per-tenant regional isolation** (e.g., a German customer who needs
   data stored only in `eu-central-1`): Clerk cannot satisfy this. Auth0 Enterprise can, but at
   significant cost. Custom JWT becomes viable if this is a deal-breaker requirement. Log as an
   open question to revisit at first enterprise customer conversation.
2. **If the SDK churn rate continues**: Clerk's breaking changes between versions add upgrade
   maintenance cost. If a v6 with a large migration lands within 6 months, re-evaluate Auth0
   as a more stable alternative.

**Not recommended — Custom JWT:** The 3–5 week implementation cost is the full engineering
bandwidth for one month. The team cannot afford to build auth instead of product at this stage.
Revisit at 50+ engineers if cost-at-scale becomes the dominant concern.

## Open Questions

These questions were unresolved at the time of writing. Each is logged in `open-questions.md`:

- **Q-2:** Does Clerk's SOC 2 Type II scope cover the Organizations feature specifically, or only
  the core auth flow? (→ relevant to compliance attestation for enterprise customers)
- **Q-3:** What is Clerk's contractual commitment on data residency — is it per-account or
  per-workspace, and can it be contractually locked to a region for an individual customer?
- **Q-4:** At what point (MAU or ARR) does the Clerk → Auth0 migration become economically
  justified purely on pricing? (→ model this when first $1M ARR milestone is in sight)

## References

- Clerk documentation: https://clerk.com/docs
- Clerk pricing: https://clerk.com/pricing
- Auth0 Organizations documentation: https://auth0.com/docs/manage-users/organizations
- Auth0 pricing calculator: https://auth0.com/pricing
- OWASP Authentication Cheat Sheet: https://cheatsheetseries.owasp.org/cheatsheets/Authentication_Cheat_Sheet.html
- OWASP JWT Security Cheat Sheet: https://cheatsheetseries.owasp.org/cheatsheets/JSON_Web_Token_for_Java_Cheat_Sheet.html
- Next.js + Clerk integration guide: https://clerk.com/docs/quickstarts/nextjs

> **Forward links:** This brief directly informed [ADR-0001](../adr/0001-example-use-clerk-for-auth.md).
> The auth flow behavior specified in [FEAT-001](../specs/FEAT-001-user-authentication.md) is
> constrained by the findings in §Clerk above.
