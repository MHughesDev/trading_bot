"""FB-CAN-028: Prometheus/Grafana canonical monitoring artifacts are loadable."""

from __future__ import annotations

import json
from pathlib import Path

import yaml
from fastapi.testclient import TestClient

import control_plane.api as api


def test_prometheus_config_and_alert_rules_parse():
    root = Path(__file__).resolve().parents[1]
    prom = yaml.safe_load((root / "infra/prometheus/prometheus.yml").read_text(encoding="utf-8"))
    assert "rule_files" in prom
    alerts_dir = root / "infra/prometheus/alerts"
    files = list(alerts_dir.glob("*.yml"))
    assert files, "expected at least one alert rule file"
    for f in files:
        yaml.safe_load(f.read_text(encoding="utf-8"))


def test_grafana_canonical_dashboard_json():
    root = Path(__file__).resolve().parents[1]
    p = root / "infra/grafana/provisioning/dashboards/json/tb-canonical-health.json"
    d = json.loads(p.read_text(encoding="utf-8"))
    assert d["uid"] == "tb-canonical-health"
    assert len(d.get("panels", [])) >= 1


def test_governance_monitoring_endpoint():
    c = TestClient(api.app)
    r = c.get("/governance/monitoring")
    assert r.status_code == 200
    body = r.json()
    assert body.get("grafana_dashboard_uid") == "tb-canonical-health"
    assert "prometheus_rules" in body


def test_governance_decision_record_endpoint():
    c = TestClient(api.app)
    r = c.get("/governance/decision-record")
    assert r.status_code == 200
    body = r.json()
    assert "decision_record" in body
