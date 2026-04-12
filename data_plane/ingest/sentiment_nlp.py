"""Optional FinBERT sentiment (transformers). When unavailable or disabled, returns neutral scores."""

from __future__ import annotations

import logging
from functools import lru_cache
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    pass

logger = logging.getLogger(__name__)


@lru_cache(maxsize=1)
def _pipeline():
    try:
        from transformers import pipeline

        return pipeline(
            "sentiment-analysis",
            model="ProsusAI/finbert",
            tokenizer="ProsusAI/finbert",
            device=-1,
        )
    except Exception:
        logger.warning("FinBERT pipeline unavailable (install trading-bot[sentiment_nlp])")
        return None


def score_headline_finbert(text: str) -> float:
    """
    Map FinBERT label/score to roughly [-1, 1]. Neutral → 0.
    """
    pipe = _pipeline()
    if pipe is None or not text.strip():
        return 0.0
    try:
        out = pipe(text[:512])[0]
        label = str(out.get("label", "")).lower()
        sc = float(out.get("score", 0.5))
        if "pos" in label:
            return sc
        if "neg" in label:
            return -sc
        return 0.0
    except Exception:
        logger.exception("FinBERT scoring failed")
        return 0.0
