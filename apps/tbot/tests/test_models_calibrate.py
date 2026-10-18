"""`tbot models calibrate` (N-4.1): calls the calibrate route and prints the wrapper."""
import json
import pathlib
import sys

import httpx
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "src"))
from tbot import cli  # noqa: E402
from tbot.client import Tbot, TbotError  # noqa: E402


def client_with(handler) -> Tbot:
    c = Tbot(base_url="http://x", token="t")
    c._client = httpx.Client(base_url="http://x", transport=httpx.MockTransport(handler))
    return c


def test_calibrate_posts_the_method_to_the_version_route():
    seen = {}

    def handler(request: httpx.Request) -> httpx.Response:
        seen["path"] = request.url.path
        seen["body"] = json.loads(request.content)
        return httpx.Response(200, json={"wrapper_version": 4, "wraps": 2, "calibration": {
            "method": "aci_cusum", "coverage": 0.9, "nominal_coverage": 0.9, "reset_indices": [60]}})

    out = client_with(handler).model_calibrate("m1", 2, method="aci_cusum", gamma=0.01)
    assert seen["path"] == "/api/models/m1/versions/2/calibrate"
    assert seen["body"] == {"method": "aci_cusum", "gamma": 0.01}
    assert out["wrapper_version"] == 4


def test_the_cli_prints_the_wrapper_and_a_refusal_keeps_its_reason(capsys):
    def ok(request):
        return httpx.Response(200, json={"wrapper_version": 4, "wraps": 2, "calibration": {
            "method": "aci", "coverage": 0.91, "nominal_coverage": 0.9, "reset_indices": []}})

    args = cli.build_parser().parse_args(["models", "calibrate", "m1", "2", "--method", "aci"])
    assert args.func(client_with(ok), args) == 0
    assert "wrapper      v4 wraps v2 (aci)" in capsys.readouterr().out

    def refuse(request):
        return httpx.Response(422, json={"error": "too_few_realised_forecasts", "message": "3 realised"})

    with pytest.raises(TbotError) as e:
        args.func(client_with(refuse), args)
    assert "too_few_realised_forecasts" in str(e.value) or e.value.status == 422
