# Risk engine

## `RiskEngine` (`risk_engine/engine.py`)

- Evaluates **`ActionProposal`** vs **`RiskState`** and settings (stale feed/data, spread, drawdown, mode, exposure, position).
- Returns **`TradeAction | None`**; `None` means blocked.
- Precedence is documented in **`docs/RISK_PRECEDENCE.MD`** (authoritative for ordering).

## Optional inputs

- **`feed_last_message_at`** — WS freshness
- **`position_signed_qty`** — Signed position for flatten / reduce-only
- **`available_cash_usd`** — Optional cap on buy notional (e.g. replay with portfolio cash when enabled in settings)

## Signing

- **`signing.py`** — HMAC signing for **`OrderIntent`** when `NM_RISK_SIGNING_SECRET` is set
- **`execution/intent_gate.py`** — Enforces that only allowed intents reach adapters

## Policy

- No bypassing risk/signing for “quick” production tests; unsigned execution only when explicitly allowed by settings (dev).
