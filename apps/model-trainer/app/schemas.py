from typing import Optional
from pydantic import BaseModel


class ProgressConfig(BaseModel):
    nats_subject: str


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


class TrainResponse(BaseModel):
    status: str  # "succeeded" | "failed"
    artifact_uri: Optional[str] = None
    sha256: Optional[str] = None
    size_bytes: Optional[int] = None
    metrics: Optional[dict] = None
    framework_version: Optional[str] = None
    error: Optional[str] = None
