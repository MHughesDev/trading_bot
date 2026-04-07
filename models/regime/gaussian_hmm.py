from __future__ import annotations

from dataclasses import dataclass, field

import numpy as np
from hmmlearn.hmm import GaussianHMM

from app.contracts.common import SemanticRegime
from app.contracts.models import RegimeOutput

DEFAULT_STATE_MAP: dict[int, SemanticRegime] = {
    0: SemanticRegime.BULL,
    1: SemanticRegime.BEAR,
    2: SemanticRegime.VOLATILE,
    3: SemanticRegime.SIDEWAYS,
}


@dataclass(slots=True)
class GaussianRegimeModel:
    n_states: int = 4
    covariance_type: str = "full"
    random_state: int = 42
    _model: GaussianHMM = field(init=False)
    _trained: bool = field(default=False, init=False)
    _state_map: dict[int, SemanticRegime] = field(default_factory=lambda: dict(DEFAULT_STATE_MAP))

    def __post_init__(self) -> None:
        self._model = GaussianHMM(
            n_components=self.n_states,
            covariance_type=self.covariance_type,
            random_state=self.random_state,
            n_iter=200,
        )

    def fit(self, x: np.ndarray) -> None:
        if x.ndim != 2:
            raise ValueError("Regime model expects 2D feature matrix")
        if x.shape[0] < self.n_states * 3:
            raise ValueError("Insufficient samples to fit HMM robustly")
        self._model.fit(x)
        self._trained = True

    def infer(self, symbol: str, features: np.ndarray) -> RegimeOutput:
        if not self._trained:
            # Cold-start fallback keeps the pipeline running while signaling low confidence.
            return RegimeOutput(
                symbol=symbol,
                raw_state=3,
                semantic_state=SemanticRegime.SIDEWAYS,
                probabilities=[0.0, 0.0, 0.0, 1.0],
                confidence=0.2,
            )

        if features.ndim == 1:
            features = features.reshape(1, -1)
        if features.ndim != 2:
            raise ValueError("Regime infer expects 1D or 2D feature matrix")

        posteriors = self._model.predict_proba(features)[-1]
        raw_state = int(np.argmax(posteriors))
        semantic = self._state_map.get(raw_state, SemanticRegime.SIDEWAYS)
        confidence = float(np.max(posteriors))
        return RegimeOutput(
            symbol=symbol,
            raw_state=raw_state,
            semantic_state=semantic,
            probabilities=[float(v) for v in posteriors.tolist()],
            confidence=confidence,
        )
