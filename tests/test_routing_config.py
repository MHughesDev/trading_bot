from app.config.settings import AppSettings


def test_execution_router_rejects_wrong_adapter():
    from execution.router import get_execution_adapter

    s = AppSettings(execution_mode="paper", execution_paper_adapter="wrong")
    try:
        get_execution_adapter(s)
        raise AssertionError("expected ValueError")
    except ValueError as e:
        assert "alpaca" in str(e).lower()
