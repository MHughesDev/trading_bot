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


class V2Row(BaseModel):
    # The caller's handle for this row, echoed back.
    key: str
    # Feature name -> value. Every feature the model was trained on must be
    # present; a row missing any abstains naming them.
    features: dict


class V2PredictRequest(BaseModel):
    model_id: str
    version: int
    model_kind: str
    artifact_uri: str
    artifact_hash: str
    rows: list[V2Row]
    # A transform's code ({language, entrypoint, source}); no artifact (U-5.7).
    transform: Optional[dict] = None


class V2Answer(BaseModel):
    key: Optional[str] = None
    # "answered" | "abstained" | "unavailable"
    status: str
    # Named outputs in plain JSON, typed by the caller against the contract.
    outputs: dict = {}
    detail: Optional[str] = None


class V2PredictResponse(BaseModel):
    model_id: str
    version: int
    answers: list[V2Answer]
    latency_ms: int


class EmbedRow(BaseModel):
    key: str
    text: str


class EmbedRequest(BaseModel):
    """`POST /v2/embed`: an embedding version's adapter and the texts to embed
    (Set U U-5.2). Each answer carries `embedding` — the provider's vector."""
    model_id: str
    version: int
    adapter: dict
    rows: list[EmbedRow]


class EmbedResponse(BaseModel):
    model_id: str
    version: int
    answers: list[V2Answer]


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
