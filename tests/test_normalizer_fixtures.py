import json
from pathlib import Path

from data_plane.ingest.normalizers import normalize_ws_message


def test_ticker_fixture_normalizes():
    p = Path(__file__).parent / "fixtures" / "coinbase_ws" / "ticker_flat.json"
    msg = json.loads(p.read_text(encoding="utf-8"))
    out = normalize_ws_message(msg)
    assert out is not None
    assert out.symbol == "BTC-USD"
    assert out.price == 50000.0
    assert out.bid is not None and out.ask is not None
