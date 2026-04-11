from policy_model.objects import PolicyAction, PolicyObservation
from policy_model.shadow import shadow_compare_actions


def test_shadow_delta():
    obs = PolicyObservation(
        forecast_features=[0.1],
        portfolio_features=[1.0],
        execution_features=[1.0],
        risk_features=[0.1],
        history_features=None,
    )

    def p(o):
        return PolicyAction(target_exposure=0.5)

    def s(o):
        return PolicyAction(target_exposure=0.2)

    out = shadow_compare_actions(obs, p, s, record_metric=False)
    assert out["abs_delta"] == 0.3
