from datetime import UTC, datetime

from data_plane.bars.rolling import RollingMinuteBars
from data_plane.features.pipeline import FeaturePipeline
from decision_engine.feature_frame import enrich_bars_last_row


def test_rolling_minute_bar_roll():
    r = RollingMinuteBars("BTC-USD")
    t0 = datetime(2025, 1, 1, 12, 0, 30, tzinfo=UTC)
    t1 = datetime(2025, 1, 1, 12, 1, 0, tzinfo=UTC)
    r.on_tick(100.0, t0, 1.0)
    r.on_tick(101.0, t0, 1.0)
    r.on_tick(99.0, t1, 2.0)
    df = r.bars_frame_with_partial()
    assert df.height >= 2
    fp = FeaturePipeline()
    row = enrich_bars_last_row(df, fp)
    assert row is not None
    assert "ret_1" in row or "rsi_14" in row
