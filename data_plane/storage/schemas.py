"""QuestDB table DDL for bars and decision traces."""

# Legacy table (pre FB-AP-014); prefer canonical_bars for new writes.
QUESTDB_BARS_DDL = """
CREATE TABLE IF NOT EXISTS bars (
  ts TIMESTAMP,
  symbol SYMBOL,
  open DOUBLE,
  high DOUBLE,
  low DOUBLE,
  close DOUBLE,
  volume DOUBLE,
  source SYMBOL
) TIMESTAMP(ts) PARTITION BY DAY;
"""

# FB-AP-014: authoritative OHLCV; ts = bucket start UTC; interval_seconds = bar width.
QUESTDB_CANONICAL_BARS_DDL = """
CREATE TABLE IF NOT EXISTS canonical_bars (
  ts TIMESTAMP,
  symbol SYMBOL,
  interval_seconds INT,
  open DOUBLE,
  high DOUBLE,
  low DOUBLE,
  close DOUBLE,
  volume DOUBLE,
  source SYMBOL,
  schema_version INT
) TIMESTAMP(ts) PARTITION BY DAY;
"""

QUESTDB_DECISIONS_DDL = """
CREATE TABLE IF NOT EXISTS decision_traces (
  ts TIMESTAMP,
  symbol SYMBOL,
  route_id SYMBOL,
  regime SYMBOL,
  action SYMBOL,
  details STRING
) TIMESTAMP(ts) PARTITION BY DAY;
"""

QUESTDB_MICROSERVICE_EVENTS_DDL = """
CREATE TABLE IF NOT EXISTS microservice_events (
  ts TIMESTAMP,
  topic SYMBOL,
  event_type SYMBOL,
  trace_id SYMBOL,
  symbol SYMBOL,
  payload STRING
) TIMESTAMP(ts) PARTITION BY DAY;
"""


async def ensure_questdb_schema(conn) -> None:
    await conn.execute(QUESTDB_BARS_DDL)
    await conn.execute(QUESTDB_CANONICAL_BARS_DDL)
    await conn.execute(QUESTDB_DECISIONS_DDL)
    await conn.execute(QUESTDB_MICROSERVICE_EVENTS_DDL)
