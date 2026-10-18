"""`run.py --suite judgement` (S-10.16).

Scoring is in Rust; these tests prove the runner's half: that it reaches the
scorer, refuses a live run it cannot do, and that the published baseline has the
(tier, workflow) shape the acceptance names. The scorer itself is covered by
`cargo test -p harness --lib -- judgement`.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "runner"))

import run  # noqa: E402

BASELINE = ROOT / "results" / "judgement-baseline.json"
TIERS = {"degraded", "local_small", "local_mid", "local_high", "frontier"}


def stub(tmp_path: Path) -> str:
    """A scorer that prints the committed baseline, standing in for the binary."""
    script = tmp_path / "scorer.py"
    script.write_text(
        f"import sys\nsys.stdout.write(open({str(BASELINE)!r}, encoding='utf-8').read())\n",
        encoding="utf-8",
    )
    return f"{Path(sys.executable).as_posix()} {script.as_posix()}"


def test_the_baseline_has_a_judgement_cell_per_tier_and_workflow():
    cards = json.loads(BASELINE.read_text(encoding="utf-8"))
    assert {c["tier"] for c in cards} == TIERS
    for card in cards:
        assert card["profile_id"].startswith("reference-policy/"), "synthetic rows say so"
        assert "not a model" in card["hardware"]
        cells = {s["suite"] for s in card["suites"] if s["suite"].startswith("judgement/")}
        assert {"judgement/investigate", "judgement/falsify", "judgement/compare"} <= cells
        assert not any(s["sealed"] for s in card["suites"]), "a PR baseline holds no sealed cell"
        assert sum(s["n"] for s in card["suites"]) >= 30


def test_a_replay_run_reaches_the_scorer_and_carries_its_cards(tmp_path, monkeypatch):
    monkeypatch.setenv("TBOT_JUDGEMENT_CMD", stub(tmp_path))
    out = tmp_path / "card.json"
    assert run.main(["--suite", "judgement", "--replay", "--out", str(out)]) == 0
    doc = json.loads(out.read_text(encoding="utf-8"))
    assert doc["synthetic_reference_policy"] is True
    assert {c["tier"] for c in doc["cards"]} == TIERS


def test_a_live_judgement_run_is_refused_not_faked(capsys):
    assert run.main(["--suite", "judgement"]) == 3
    err = capsys.readouterr().err
    assert "H-2" in err and "credential" in err


def test_a_failing_scorer_fails_the_run(tmp_path, monkeypatch):
    bad = tmp_path / "bad.py"
    bad.write_text("import sys\nsys.exit(2)\n", encoding="utf-8")
    monkeypatch.setenv("TBOT_JUDGEMENT_CMD", f"{Path(sys.executable).as_posix()} {bad.as_posix()}")
    assert run.main(["--suite", "judgement", "--replay"]) == 2


def test_judgement_cannot_be_mixed_with_a_python_suite():
    assert run.main(["--suite", "judgement", "--suite", "noise", "--replay"]) == 2


@pytest.mark.parametrize("name", ["judgement"])
def test_the_suite_is_declared_with_its_cadence(name):
    suites = run.load_suites()["suites"]
    assert name in suites
    assert set(suites[name]["cadence"]) == {"pr_smoke", "nightly"}
