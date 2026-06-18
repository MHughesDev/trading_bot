from typing import Optional
from pydantic import BaseModel


class ProgressConfig(BaseModel):
    nats_subject: str


class FoldSpec(BaseModel):
    """Walk-forward fold index ranges, Rust-computed (I-0.10 / ADR-0017).

    The sidecar slices the materialized Parquet by these half-open index ranges
    and trains/scores per fold without picking its own split.
    """
    index: int
    train_start: int
    train_end: int
    cal_start: int
    cal_end: int
    test_start: int
    test_end: int


class TrainRequest(BaseModel):
    run_id: str
    model_id: str
    model_kind: str
    framework: str
    runtime: str = "python"
    definition: dict
    dataset_uri: str
    dataset_hash: str
    output_prefix: str
    progress: ProgressConfig
    # `data` is kept for backward-compat but no longer used when folds are
    # present — the sidecar reads from the pre-materialized Parquet (I-0.8).
    data: Optional[dict] = None
    # Walk-forward fold index ranges from Rust (I-0.10). When present, the
    # sidecar uses these ranges to slice the Parquet; when absent it falls back
    # to the ordinal split_indices path.
    folds: Optional[list[FoldSpec]] = None


class TrainResponse(BaseModel):
    status: str  # "succeeded" | "failed"
    artifact_uri: Optional[str] = None
    sha256: Optional[str] = None
    size_bytes: Optional[int] = None
    metrics: Optional[dict] = None
    framework_version: Optional[str] = None
    error: Optional[str] = None


# ---------------------------------------------------------------------------
# Eval schemas (I-2.1)
# ---------------------------------------------------------------------------

class EvalRequest(BaseModel):
    eval_id: str
    model_id: str
    version: int
    model_kind: str
    artifact_uri: str
    artifact_hash: str
    dataset_uri: str    # pre-materialized Parquet (test window)
    dataset_hash: str
    definition: dict
    trial_count: int = 0
    holdout_used: bool = False
    run_baselines: bool = True
    folds: Optional[list[FoldSpec]] = None
    progress: Optional[ProgressConfig] = None


class EvalResponse(BaseModel):
    status: str  # "succeeded" | "failed"
    metrics: Optional[dict] = None
    scorecard: Optional[dict] = None
    report: Optional[dict] = None
    error: Optional[str] = None


