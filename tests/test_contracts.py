from __future__ import annotations

import pytest
from pydantic import ValidationError

from app.contracts.common import DataSource
from app.contracts.events import BarEvent


def test_bar_event_requires_schema_fields() -> None:
    evt = BarEvent(
        symbol="BTC-USD",
        open=100.0,
        high=101.0,
        low=99.0,
        close=100.5,
        volume=10.0,
        source=DataSource.COINBASE,
    )
    assert evt.schema_version == "v1"


def test_bar_event_rejects_extra_field() -> None:
    with pytest.raises(ValidationError):
        BarEvent(
            symbol="BTC-USD",
            open=100.0,
            high=101.0,
            low=99.0,
            close=100.5,
            volume=10.0,
            source=DataSource.COINBASE,
            unexpected="x",
        )
