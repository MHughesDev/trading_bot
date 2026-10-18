-- 0191 — the Kalshi candlestick-history source (Set P P-3.4).
--
-- `collectors::prediction::history::SOURCE_NAME = "kalshi_history"` was declared in
-- code and has had no `dataplane.source` row, so nothing it wrote could be
-- attributed to a source and `cargo xtask check-free-sources` could not see it.
--
-- Not the same row as `kalshi_rest` (id 13, the live quote poller, whose rows carry a
-- real receipt): every row this source produces is derived, knowledge time = the
-- period's close + the declared lag, flagged BACKFILLED_KNOWLEDGE_TIME. Collapsing
-- the two would make a backfilled probability indistinguishable from an observed one
-- (D-P3).

INSERT INTO dataplane.source (source_id, name, kind, declared_vendor_lag_ms, notes, cost_class) VALUES
  (24, 'kalshi_history', 'rest_history', 60000,
       'Kalshi candlesticks (public, unauthenticated GET). A candlestick describes a closed period; it is readable one period after it closes, declared as 60 s. Every row is derived and flagged; the live poller (kalshi_rest) is the observed series.',
       'free')
ON CONFLICT (source_id) DO NOTHING;

UPDATE dataplane.source SET cost_class = 'free' WHERE name = 'kalshi_history' AND cost_class IS NULL;
