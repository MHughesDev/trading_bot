-- 0141 — paper trading runs in the app (ADR-0048, owner decision 2026-10-02).
--
-- 0105 constrained a forward deployment's venue to 'alpaca_paper': Alpaca's
-- paper account was to be the paper venue. The owner has decided paper trading
-- happens on the platform's own paper venue instead — the same venue core the
-- backtests run on (ADR-0044) — and never on a broker's paper account.
--
-- The constraint keeps its shape: one value, so widening it is still a
-- migration, a review and roadmap gate H-4. Only the value changes.
--
-- No row can hold 'alpaca_paper': `mlops.paper_deployment` has never had a
-- writer (Set T T-0.4). Should one exist anyway, ADD CONSTRAINT fails and this
-- migration stops loudly rather than rewriting history in an append-only table.

ALTER TABLE mlops.paper_deployment
  DROP CONSTRAINT IF EXISTS paper_deployment_venue_check;

ALTER TABLE mlops.paper_deployment
  ADD CONSTRAINT paper_deployment_venue_check
  CHECK (venue IN ('platform_paper'));
