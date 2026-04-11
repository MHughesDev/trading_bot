"""FB-FR-PG9: simulated forecast build failure surfaces as abstention, not silent success."""

from forecaster_model.inference.robust import safe_build_forecast_packet


def test_fault_injection_no_silent_packet():
    pkt, reasons = safe_build_forecast_packet(lambda: (_ for _ in ()).throw(ValueError("gpu oom")))
    assert pkt is None
    assert reasons
