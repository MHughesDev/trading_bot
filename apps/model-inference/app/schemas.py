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
