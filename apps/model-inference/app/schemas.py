from typing import Optional
from pydantic import BaseModel


class Instance(BaseModel):
    instrument_id: str
    features: dict  # feature_name -> float


class PredictRequest(BaseModel):
    model_id: str
    version: int
    model_kind: str
    artifact_uri: str
    artifact_hash: str
    instances: list[Instance]


class Forecast(BaseModel):
    direction: str  # "up"|"down"|"flat"
    magnitude: str  # decimal string (ADR-0002)
    confidence: float
    horizon: str
    # Distribution fields (ADR-0016) — absent for point/classification models.
    quantile_levels: Optional[list[float]] = None
    quantiles_return: Optional[list[float]] = None
    median_return: Optional[float] = None
    sigma: Optional[float] = None


class PredictResponse(BaseModel):
    model_id: str
    version: int
    predictions: list[Forecast]
    latency_ms: int


class LLMPredictRequest(BaseModel):
    model_id: str
    version: int
    adapter: dict
    prompt: str
    params: dict = {}


class LLMPredictResponse(BaseModel):
    text: str
    tokens: int
    latency_ms: int
    cost_usd: str
    trace_id: str
