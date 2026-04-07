"""4-state Gaussian HMM regime model with semantic mapping (bull, bear, volatile, sideways)."""

from __future__ import annotations

import logging
from typing import Any

import numpy as np
from hmmlearn.hmm import GaussianHMM
from sklearn.preprocessing import StandardScaler

from app.contracts.regime import RegimeOutput, SemanticRegime

logger = logging.getLogger(__name__)

STATE_SEMANTICS: list[SemanticRegime] = [
    SemanticRegime.BULL,
    SemanticRegime.BEAR,
    SemanticRegime.VOLATILE,
    SemanticRegime.SIDEWAYS,
]


class GaussianHMMRegimeModel:
    """Trained on return/vol feature matrix; maps latent states to fixed semantics by validation order."""

    def __init__(self, n_states: int = 4, seed: int = 42) -> None:
        if n_states != 4:
            raise ValueError("V1 spec requires 4 HMM states")
        self._hmm = GaussianHMM(
            n_components=n_states,
            covariance_type="full",
            n_iter=200,
            random_state=seed,
        )
        self._scaler = StandardScaler()
        self._fitted = False
        self._state_order: list[int] | None = None

    def fit(self, X: np.ndarray) -> None:
        """X shape (n_samples, n_features), e.g. [ret_1, vol_5, ...]."""
        if X.shape[0] < self._hmm.n_components * 10:
            logger.warning("HMM fit: few samples (%s), regime may be unstable", X.shape[0])
        Xs = self._scaler.fit_transform(X)
        self._hmm.fit(Xs)
        self._fitted = True
        self._state_order = list(range(self._hmm.n_components))

    def predict_proba_last(self, X: np.ndarray) -> RegimeOutput:
        if not self._fitted:
            return RegimeOutput(
                state_index=3,
                semantic=SemanticRegime.SIDEWAYS,
                probabilities=[0.25, 0.25, 0.25, 0.25],
                confidence=0.0,
            )
        Xs = self._scaler.transform(X)
        last = Xs[-1:]
        _log_prob, posteriors = self._hmm.score_samples(last)
        p = posteriors[-1]
        state = int(np.argmax(p))
        conf = float(np.max(p))
        sem = STATE_SEMANTICS[state % len(STATE_SEMANTICS)]
        return RegimeOutput(
            state_index=state,
            semantic=sem,
            probabilities=[float(x) for x in p],
            confidence=conf,
        )

    def serialize(self) -> dict[str, Any]:
        return {
            "n_states": self._hmm.n_components,
            "fitted": self._fitted,
        }
