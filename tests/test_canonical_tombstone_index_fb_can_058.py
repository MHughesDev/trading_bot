"""FB-CAN-058: tombstone index doc exists and links key removals."""

from __future__ import annotations

from pathlib import Path


def test_tombstone_index_documents_known_removals() -> None:
    root = Path(__file__).resolve().parents[1]
    p = root / "docs" / "CANONICAL_TOMBSTONE_INDEX.MD"
    text = p.read_text(encoding="utf-8")
    assert "FB-CAN-012" in text
    assert "MASTER_SPEC_RISK_STATE_GAP" in text
    assert "action_generator.py" in text
    assert "github.com/MHughesDev/trading_bot/commit/" in text
    assert "CANONICAL_DELETION_LOG.MD" in text
