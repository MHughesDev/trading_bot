"""`tbot.models.ModelInterface` (N-2.9): abstract, importable, and identical to the trainer's."""
import pathlib
import sys

import numpy as np
import pandas as pd
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "src"))
from tbot.models import ModelInterface, UndeclaredFeature, strict_frame  # noqa: E402


def test_the_interface_cannot_be_instantiated_and_each_method_is_required():
    with pytest.raises(TypeError):
        ModelInterface()

    class Partial(ModelInterface):
        def fit(self, X, y, sample_weight=None): ...
        def predict(self, X): ...
        def save(self, path): ...  # no load

    with pytest.raises(TypeError):
        Partial()


def test_a_complete_model_round_trips_through_save_and_load(tmp_path):
    class Mean(ModelInterface):
        def fit(self, X, y, sample_weight=None):
            self.m = float(np.mean(y))

        def predict(self, X):
            return np.full(len(X), self.m)

        def save(self, path):
            (pathlib.Path(path) / "m.txt").write_text(str(self.m))

        @classmethod
        def load(cls, path):
            o = cls()
            o.m = float((pathlib.Path(path) / "m.txt").read_text())
            return o

    m = Mean()
    m.fit(pd.DataFrame({"a": [1, 2]}), np.array([1.0, 3.0]))
    m.save(str(tmp_path))
    assert Mean.load(str(tmp_path)).predict(pd.DataFrame({"a": [0]}))[0] == 2.0


def test_a_strict_frame_names_an_undeclared_feature():
    f = strict_frame(pd.DataFrame({"a": [1.0]}))
    with pytest.raises(UndeclaredFeature, match="b"):
        f["b"]


def test_the_copy_is_identical_to_the_trainers_interface():
    here = pathlib.Path(__file__).resolve()
    trainer = here.parents[2] / "model-trainer" / "app" / "byo_interface.py"
    if not trainer.exists():
        pytest.skip("trainer source not in this checkout")
    body = lambda p: p.read_text(encoding="utf-8").split('"""', 2)[2]  # noqa: E731
    assert body(here.parents[1] / "src" / "tbot" / "models.py") == body(trainer)
