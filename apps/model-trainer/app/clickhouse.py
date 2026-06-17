# This module has been removed as part of Set I Phase 0 (I-0.8 / ADR-0017).
#
# The trainer sidecar no longer queries ClickHouse directly. Rust materializes
# all training data through the leakage-safe DataView (I-0.4 / I-0.5), writes
# a pinned Parquet snapshot to the artifact store, and hands the sidecar the
# pre-windowed Parquet URI via TrainDispatchRequest.dataset_uri. The sidecar
# reads that Parquet and slices it using Rust-computed fold indices (I-0.10).
#
# Any import of `fetch_bars` from this module is a bug — the caller should be
# updated to read from the Parquet path in worker.py instead.

raise ImportError(
    "clickhouse.py has been removed (Set I I-0.8): the sidecar must not "
    "issue its own bar queries. Read from the pre-materialized Parquet "
    "(dataset_uri) instead."
)
