"""Unit tests for FB-AP-011 / FB-AP-012 init helpers."""

from __future__ import annotations

from pathlib import Path

from app.contracts.asset_model_manifest import AssetModelManifest
from app.runtime import asset_model_registry as reg
from orchestration.init_policy_artifacts import run_init_policy_mlp
from orchestration.init_register_manifest import register_init_artifacts_manifest


def test_run_init_policy_mlp_deterministic_seed(tmp_path: Path) -> None:
    a = run_init_policy_mlp(run_dir=tmp_path, symbol="BTC-USD", job_id="j1")
    b = run_init_policy_mlp(run_dir=tmp_path / "other", symbol="BTC-USD", job_id="j1")
    assert a["policy_seed"] == b["policy_seed"]
    assert Path(a["policy_mlp_path"]).is_file()


def test_register_init_artifacts_manifest(tmp_path: Path, monkeypatch) -> None:
    man_dir = tmp_path / "m"
    man_dir.mkdir(parents=True, exist_ok=True)
    monkeypatch.setattr(reg, "_DEFAULT_DIR", man_dir)
    run_dir = tmp_path / "run"
    (run_dir / "forecaster").mkdir(parents=True)
    (run_dir / "policy").mkdir(parents=True)
    (run_dir / "forecaster" / "forecaster_torch.pt").write_text("x")
    (run_dir / "policy" / "policy_mlp.npz").write_bytes(b"npz")
    out = register_init_artifacts_manifest(symbol="SOL-USD", job_id="jid", run_dir=run_dir)
    assert Path(out["manifest_path"]).is_file()
    m = reg.load_manifest("SOL-USD")
    assert m is not None
    assert isinstance(m, AssetModelManifest)
    assert m.forecaster_torch_path
    assert m.policy_mlp_path
