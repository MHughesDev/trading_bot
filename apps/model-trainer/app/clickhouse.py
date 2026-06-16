"""ClickHouse access for the trainer sidecar.

Fetches deduplicated OHLCV bars over the ClickHouse HTTP interface, matching the
Rust `BarStore::load_bars` read path (latest revision wins per `available_time`).
The connection URL is read from the `CLICKHOUSE_URL` environment variable, which
docker-compose sets to the container-reachable address
(`http://user:pass@clickhouse:8123/db`).
"""

import io
import os
from urllib.parse import urlparse

import httpx
import pandas as pd


def _parse_url(url: str) -> tuple[str, str | None, str | None, str]:
    """Return (base_url, user, password, database) from a ClickHouse URL."""
    parsed = urlparse(url)
    scheme = parsed.scheme or "http"
    host = parsed.hostname or "localhost"
    port = parsed.port or 8123
    base = f"{scheme}://{host}:{port}"
    database = parsed.path.lstrip("/") or "default"
    return base, parsed.username, parsed.password, database


def fetch_bars(
    instrument_id: str,
    timeframe: str,
    start_iso: str,
    end_iso: str,
) -> pd.DataFrame:
    """Fetch deduplicated bars for one instrument as a DataFrame.

    Columns: ts_ms, open, high, low, close, volume, trade_count (numeric),
    ordered by time ascending. Raises on transport/HTTP error.
    """
    url = os.environ.get("CLICKHOUSE_URL", "http://trading:trading@clickhouse:8123/trading")
    base, user, password, database = _parse_url(url)

    # Parameterized query (ClickHouse {name:Type} substitution) — no string
    # interpolation of user-controlled values into the SQL text.
    sql = (
        "SELECT toUnixTimestamp64Milli(available_time) AS ts_ms, "
        "toFloat64(argMax(open, revision)) AS open, "
        "toFloat64(argMax(high, revision)) AS high, "
        "toFloat64(argMax(low, revision)) AS low, "
        "toFloat64(argMax(close, revision)) AS close, "
        "toFloat64(argMax(volume, revision)) AS volume, "
        "argMax(trade_count, revision) AS trade_count "
        "FROM market_bars "
        "WHERE instrument_id = {inst:String} AND timeframe = {tf:String} "
        "AND available_time >= parseDateTime64BestEffort({start:String}) "
        "AND available_time < parseDateTime64BestEffort({end:String}) "
        "GROUP BY available_time ORDER BY ts_ms "
        "FORMAT JSONEachRow"
    )

    params = {
        "database": database,
        "param_inst": instrument_id,
        "param_tf": timeframe,
        "param_start": start_iso,
        "param_end": end_iso,
    }
    auth = (user, password) if user is not None else None

    with httpx.Client(timeout=120.0) as client:
        resp = client.post(base, params=params, content=sql, auth=auth)
        resp.raise_for_status()
        text = resp.text.strip()

    if not text:
        return pd.DataFrame(
            columns=["ts_ms", "open", "high", "low", "close", "volume", "trade_count"]
        )

    return pd.read_json(io.StringIO(text), lines=True)
