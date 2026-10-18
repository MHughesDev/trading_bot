-- 0227 -- a served forecast records the horizon and bar timeframe it was made for (Set N N-4.1).
--
-- model_forecasts.horizon_bars existed and was never written, and there was no timeframe, so a
-- forecast could not be joined to the bars that realise it. The realisation pass
-- (forecast_realisation::realise_pending) needs both; a row without them is counted as
-- unresolvable and left alone. The grant on the table to app_role (0222) covers new columns.

ALTER TABLE model_forecasts ADD COLUMN IF NOT EXISTS timeframe TEXT;

CREATE INDEX IF NOT EXISTS idx_model_forecasts_unrealised
    ON model_forecasts (created_at) WHERE realized IS NULL;
