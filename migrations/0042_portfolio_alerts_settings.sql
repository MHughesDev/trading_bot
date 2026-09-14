-- Portfolio history, price alerts and per-user interface settings.
--
-- Three UI surfaces shipped ahead of their backend and were showing an honest
-- "awaiting data" state. This migration gives each of them somewhere to live:
--
--   equity_snapshots  the dashboard equity curve. The paper engine knows equity
--                     at any instant but nothing was recording it over time, so
--                     the curve had no series to draw. A sampler writes one row
--                     per asset class per interval; the endpoint reads them back.
--
--   price_alerts      alerts were held in localStorage, which means they are lost
--                     on a different device and cannot fire while the tab is shut.
--
--   user_settings     risk limits in particular need to outlive one browser. The
--                     payload is a JSON blob so the interface can add a preference
--                     without a migration; the columns that the *server* enforces
--                     are promoted out of it as they start being enforced.

CREATE TABLE IF NOT EXISTS equity_snapshots (
  id            BIGSERIAL PRIMARY KEY,
  at            TIMESTAMPTZ NOT NULL DEFAULT now(),
  -- 'paper' | 'live'. Kept as text rather than an enum so a third environment
  -- (e.g. a shadow account) does not need a type migration.
  account_mode  TEXT        NOT NULL,
  asset_class   TEXT        NOT NULL,
  currency      TEXT        NOT NULL,
  equity        NUMERIC     NOT NULL,
  cash          NUMERIC     NOT NULL,
  used_margin   NUMERIC     NOT NULL,
  realized_pnl  NUMERIC     NOT NULL,
  unrealized_pnl NUMERIC    NOT NULL,
  fees_paid     NUMERIC     NOT NULL,
  open_positions INTEGER    NOT NULL DEFAULT 0,
  -- External cash movement recorded at this instant, if any. Drives the deposit
  -- markers on the equity curve, which is what separates "made money" from
  -- "was given money".
  cash_flow     NUMERIC     NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS equity_snapshots_at_idx
  ON equity_snapshots (account_mode, at DESC);
CREATE INDEX IF NOT EXISTS equity_snapshots_class_idx
  ON equity_snapshots (account_mode, asset_class, at DESC);

CREATE TABLE IF NOT EXISTS price_alerts (
  alert_id      UUID PRIMARY KEY,
  user_id       UUID        NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
  instrument_id TEXT        NOT NULL,
  -- price_above | price_below | pct_change | indicator_cross
  kind          TEXT        NOT NULL,
  value         NUMERIC     NOT NULL,
  note          TEXT,
  active        BOOLEAN     NOT NULL DEFAULT TRUE,
  -- in-app | email, as a comma-separated list; a set table is overkill for two.
  channels      TEXT        NOT NULL DEFAULT 'inapp',
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  triggered_at  TIMESTAMPTZ,
  -- The mark that tripped it, so a fired alert can say why.
  triggered_price NUMERIC
);

CREATE INDEX IF NOT EXISTS price_alerts_user_idx
  ON price_alerts (user_id, active, instrument_id);

CREATE TABLE IF NOT EXISTS user_settings (
  user_id    UUID PRIMARY KEY REFERENCES users(user_id) ON DELETE CASCADE,
  -- The whole interface preference payload. Device-local preferences stay in
  -- the browser; this is the copy that follows the user between devices.
  payload    JSONB       NOT NULL DEFAULT '{}'::jsonb,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
