# Risk engine

> **Architecture status:** This file describes the **repository as implemented today**. For **target** system architecture during the canonical APEX migration, see [`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) and [`../Human Provided Specs/new_specs/canonical/`](../Human%20Provided%20Specs/new_specs/canonical/). Where this document conflicts with canonical APEX specs on **future** design, treat this file as **as-built** only, not the roadmap.

> **Tombstone (FB-CAN-023):** As-built risk path. Canonical risk/sizing **requirements** → **[`CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD)**, **[`CANONICAL_PRECEDENCE.MD`](CANONICAL_PRECEDENCE.MD)**, [`docs/architecture/risk_precedence.md`](../architecture/risk_precedence.md).

## `RiskEngine` (`risk_engine/engine.py`)

- Evaluates **`ActionProposal`** vs **`RiskState`** and settings (stale feed/data, spread, drawdown, mode, exposure, position).
- **FB-CAN-007:** Notional for new entries uses **`risk_engine/canonical_sizing.py`** — layered multipliers (degradation, position inertia / flip penalty, capped asymmetry boost ≤1.2×, liquidation offense/defense mode, edge-budget, concentration) before quantity sizing. **`RiskState.last_risk_sizing`** holds explainable diagnostics. Hard gates above still apply first.
- Returns **`TradeAction | None`**; `None` means blocked.
- Precedence is documented in [`docs/architecture/risk_precedence.md`](../architecture/risk_precedence.md) (authoritative for ordering); see also [`docs/RISK_PRECEDENCE.MD`](../RISK_PRECEDENCE.MD) (redirect stub).

## Optional inputs

- **`feed_last_message_at`** — WS freshness
- **`position_signed_qty`** — Signed position for flatten / reduce-only
- **`available_cash_usd`** — Optional cap on buy notional (e.g. replay with portfolio cash when enabled in settings)
- **`portfolio_equity_usd`** — Used for concentration / inertia proxies in canonical sizing (defaults to engine equity if omitted)

## Signing

- **`signing.py`** — HMAC signing for **`OrderIntent`** when `NM_RISK_SIGNING_SECRET` is set
- **`execution/intent_gate.py`** — Enforces that only allowed intents reach adapters

## Policy

- No bypassing risk/signing for “quick” production tests; unsigned execution only when explicitly allowed by settings (dev).
