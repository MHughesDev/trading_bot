# Example ADR — Delete This File After Reading

> This file demonstrates the ADR format end-to-end, including: a well-populated Context section
> that explains *why* the decision was forced (not just what was decided), a Rationale section
> that explains the trade-offs consciously accepted, at least two substantive alternatives with
> real reasons for rejection, forward links to the spec and research that informed it, and a
> re-derivation trigger note at the bottom showing how a status change would propagate.

---

# ADR-0001: Use Clerk for Authentication

**Status:** Accepted
**Date:** 2024-03-08
**Deciders:** Engineering team (2 engineers)

## Context

The task management app requires user authentication with multi-tenant workspace isolation from
day one. The following forces drove this decision:

- **Team size and timeline:** 2 engineers, 6-month launch timeline. Auth is load-bearing
  infrastructure but not a product differentiator — time spent building it is time not spent
  on the features users pay for.
- **Multi-tenancy requirement:** Each paying customer gets an isolated workspace. A user must
  not be able to see or access resources belonging to a different workspace. This eliminates
  the simplest single-tenant auth patterns.
- **EU data residency:** At least one pilot customer is EU-based and subject to GDPR. The auth
  provider must be able to store user data in an EU region.
- **Email/password + OAuth at launch:** Both are required per the artifact. Passwordless and
  SSO/SAML are explicitly out of scope for the initial launch.
- **Cost sensitivity:** The project is pre-revenue at launch. A $240+/month minimum auth bill
  materially affects the runway calculation.

Three options were formally evaluated. See
[research/example-auth-provider-comparison.md](../research/example-auth-provider-comparison.md)
for the full analysis.

## Decision

Use Clerk as the authentication and session management provider for the initial launch.

## Rationale

Clerk satisfies every launch requirement and eliminates the largest engineering risk:

1. **Multi-tenancy on the free tier:** Clerk's Organizations feature is available without a
   paid plan. Auth0 requires $240/month minimum for the same capability. At pre-revenue stage,
   this difference is significant.

2. **Integration time verified by proof of concept:** A 2-hour proof of concept confirmed that
   a working sign-up flow with protected routes can be built in approximately 2 days on the
   current Next.js stack. Auth0's documented integration path is estimated at 5–8 days.
   The 3–6 day difference is material against a 6-month timeline.

3. **EU data residency on Pro tier:** Clerk's Pro plan provides EU data residency at the
   account level. This satisfies the GDPR requirement for the initial EU customer cohort.
   The per-tenant regional isolation limitation (all workspaces share one region setting) is
   accepted as a known constraint — see Consequences below.

4. **Eliminates custom implementation risk:** A custom JWT implementation was estimated at
   3–5 weeks and 12+ OWASP attack surfaces to get right. The team cannot spend a full
   engineering month on auth before building any product features. The security and compliance
   risk of a rushed custom implementation outweighs the long-term cost of vendor dependency.

The trade-offs consciously accepted are: Clerk SDK churn risk (breaking changes between major
versions have been documented), per-tenant regional isolation unavailable (accepted for launch,
flagged as a risk for enterprise customers), and vendor lock-in at medium migration cost
(estimated 2–3 weeks to migrate out if needed).

## Consequences

**Positive:**
- Auth integration takes ~2 days, freeing the team to build product features
- Multi-tenancy, session management, email verification, and OAuth are fully managed
- SOC 2 Type II compliance is inherited for the auth layer (reducing audit scope)
- EU data residency satisfies the GDPR requirement for the launch pilot

**Negative:**
- Vendor lock-in: migrating away from Clerk requires rebuilding the entire auth integration
  layer (~2–3 weeks estimated). The longer Clerk is in production, the harder migration becomes.
- Clerk SDK breaking changes add upgrade maintenance cost. Major version upgrades (v3→v4, v4→v5)
  have required non-trivial migration work historically.
- Per-tenant regional isolation is not available: all workspaces in the account share a single
  data residency region. A future customer requiring per-tenant isolation cannot be served by
  the current setup without migrating to Auth0 Enterprise or a custom solution.
- Pricing scales linearly with MAU: at 100k MAU, Clerk costs ~$350/month vs. $0 for custom.
  This becomes meaningful at scale but is not a concern for the initial launch phase.

**Neutral:**
- All session token issuance, rotation, and revocation is Clerk's responsibility, not the app's.
  The app trusts Clerk's JWT and does not implement its own session layer (see FEAT-001 §7.2.A).
- The app's auth-related test coverage is limited to integration behavior (redirects, protected
  routes, workspace isolation) rather than token internals.

## Alternatives Considered

### Option A: Auth0

Auth0 is a mature, enterprise-grade platform with a strong track record. It was rejected for
the initial launch for two reasons:

1. **Cost:** Multi-tenancy (Organizations) requires the Pro plan at $240/month minimum. At
   pre-revenue stage with no guaranteed MAU, committing to $240/month for auth alone was
   not justified when Clerk provides the same capability for free.

2. **Integration time:** Auth0's Next.js integration is significantly more verbose than Clerk's.
   The estimated 5–8 day integration time vs. Clerk's 2-day verified estimate represents a
   meaningful difference on a constrained timeline.

Auth0 should be re-evaluated if: (a) enterprise customers with SSO/SAML requirements emerge,
(b) per-tenant regional isolation becomes a deal requirement, or (c) Clerk's pricing at scale
(100k+ MAU) materially increases and Auth0 becomes cost-competitive by comparison.

### Option B: Custom JWT Implementation

A custom implementation was estimated at 3–5 weeks to reach a production-safe baseline and
would expand the team's SOC 2 audit scope. The OWASP Authentication Cheat Sheet identifies
12+ distinct attack surfaces (token storage, rotation, revocation, PKCE, refresh flow,
brute-force protection, etc.) that must each be addressed correctly.

For a 2-person team 6 months from launch, a 3–5 week auth build was not viable. Rejected on
timeline and security risk grounds. Revisit at 50+ engineers if pricing-at-scale becomes the
dominant driver and the team can staff a dedicated platform function.

## References

- Research brief: [research/example-auth-provider-comparison.md](../research/example-auth-provider-comparison.md)
- Spec informed by this decision: [specs/FEAT-001-user-authentication.md](../specs/FEAT-001-user-authentication.md)
- Open questions created by this decision: [open-questions.md](../open-questions.md) — Q-2, Q-3, Q-4

> **Re-derivation note (for template readers):** If this ADR's status changed to `Superseded`
> (e.g., a new ADR-0004 adopted Auth0 for enterprise), the re-derivation trigger in
> `procedures/add-adr.md` would require opening every plan listing this ADR in its Derived From
> section and setting `Derivation Status: STALE — ADR-0001 superseded YYYY-MM-DD`.
