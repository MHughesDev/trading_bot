"""FB-CAN-040 — canonical glossary file and doc references."""

from __future__ import annotations

from pathlib import Path


def test_glossary_file_exists():
    root = Path(__file__).resolve().parents[1]
    gloss = root / "docs" / "CANONICAL_GLOSSARY.MD"
    assert gloss.is_file()
    text = gloss.read_text(encoding="utf-8")
    assert "ForecastPacket" in text
    assert "PolicySystem" in text


def test_readme_and_spec_index_reference_glossary():
    root = Path(__file__).resolve().parents[1]
    readme = (root / "README.md").read_text(encoding="utf-8")
    idx = (root / "docs" / "CANONICAL_SPEC_INDEX.MD").read_text(encoding="utf-8")
    assert "CANONICAL_GLOSSARY.MD" in readme
    assert "CANONICAL_GLOSSARY.MD" in idx
