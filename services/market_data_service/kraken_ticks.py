"""Build normalized tick envelopes from Kraken ticker snapshots (microservice bus)."""

from __future__ import annotations

from datetime import UTC, datetime

from data_plane.ingest.normalizers import TickerSnapshot
from shared.messaging.envelope import EventEnvelope
from shared.messaging.trace import new_trace_id


def ticker_to_normalized_tick_envelope(
    snap: TickerSnapshot,
    *,
    trace_id: str | None = None,
    producer_service: str = "market_data_service",
    direction: int = 1,
    size_fraction: float = 0.1,
    route_id: str = "SCALPING",
) -> EventEnvelope:
    """
    Map a Kraken-normalized ticker to the payload shape expected by feature_service
    (mid/close, spread_bps, optional bid/ask for microstructure).
    """
    bid = snap.bid
    ask = snap.ask
    spread_bps = 5.0
    if bid is not None and ask is not None and bid > 0 and ask >= bid:
        mid = (bid + ask) / 2.0
        spread_bps = float((ask - bid) / mid * 10_000.0)
    else:
        mid = float(snap.price)

    sym = snap.symbol.strip()
    ts = snap.time if snap.time.tzinfo else snap.time.replace(tzinfo=UTC)

    payload: dict = {
        "symbol": sym,
        "mid_price": mid,
        "price": mid,
        "direction": int(direction),
        "size_fraction": float(size_fraction),
        "route_id": str(route_id),
        "spread_bps": spread_bps,
        "bid": bid,
        "ask": ask,
        "source": "kraken_ws",
        "data_timestamp": ts.isoformat(),
    }
    return EventEnvelope(
        event_type="market.tick.normalized",
        event_version="v1",
        trace_id=trace_id or new_trace_id(),
        producer_service=producer_service,
        symbol=sym,
        partition_key=sym,
        payload=payload,
    )


def heartbeat_envelope(
    symbols: list[str],
    *,
    last_tick_at: datetime | None,
    trace_id: str | None = None,
) -> EventEnvelope:
    now = datetime.now(UTC)
    return EventEnvelope(
        event_type="market.heartbeat",
        event_version="v1",
        trace_id=trace_id or new_trace_id(),
        producer_service="market_data_service",
        symbol=None,
        payload={
            "symbols": list(symbols),
            "last_tick_at": (last_tick_at or now).isoformat() if last_tick_at else None,
            "emitted_at": now.isoformat(),
        },
    )
