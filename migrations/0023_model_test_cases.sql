-- AI Model Studio: saved Test Lab inputs for replay and regression.

CREATE TABLE IF NOT EXISTS model_test_cases (
    case_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    model_id    TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    input_json  JSONB NOT NULL,
    expected_json JSONB,
    created_by  UUID NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_model_test_cases_model ON model_test_cases(model_id, created_at DESC);
