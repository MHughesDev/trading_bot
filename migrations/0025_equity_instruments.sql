-- Phase 6: Seed equity instrument metadata.
--
-- Seeds AAPL, SPY, and MSFT into the instruments table with NYSE session
-- schedule, haltable policy, and Regulated trust tier.
-- Asset class "equity" — same schema, different metadata, no core code branch.

INSERT INTO instruments (
    instrument_id,
    asset_class,
    venue_id,
    base_precision,
    quote_precision,
    tick_size,
    lot_size,
    trading_hours_json,
    halt_policy,
    trust_tier,
    watermark_secs,
    active
) VALUES
(
    'AAPL',
    'equity',
    'alpaca',
    0,
    2,
    0.01,
    1,
    '{"timezone":"America/New_York","sessions":[{"open":"09:30","close":"16:00"}],"has_pre_market":true,"has_post_market":true}',
    'haltable',
    'regulated',
    5,
    true
),
(
    'SPY',
    'equity',
    'alpaca',
    0,
    2,
    0.01,
    1,
    '{"timezone":"America/New_York","sessions":[{"open":"09:30","close":"16:00"}],"has_pre_market":true,"has_post_market":true}',
    'haltable',
    'regulated',
    5,
    true
),
(
    'MSFT',
    'equity',
    'alpaca',
    0,
    2,
    0.01,
    1,
    '{"timezone":"America/New_York","sessions":[{"open":"09:30","close":"16:00"}],"has_pre_market":true,"has_post_market":true}',
    'haltable',
    'regulated',
    5,
    true
)
ON CONFLICT (instrument_id) DO NOTHING;
