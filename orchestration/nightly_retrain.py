"""
Nightly retrain flow — Prefect + MLflow (optional).

Spec: no auto model promotion; manual gate after evaluation.
"""

from __future__ import annotations

import logging

logger = logging.getLogger(__name__)


def nightly_flow_stub() -> None:
    """Placeholder: fetch data → retrain → evaluate → log to MLflow → await promotion."""
    logger.info("nightly retrain stub — wire Prefect deployment in production")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO)
    nightly_flow_stub()
