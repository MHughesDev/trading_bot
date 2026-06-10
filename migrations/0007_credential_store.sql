-- Encrypted credential store for per-user venue credentials (Phase 1).
-- Fills the sequence gap between 0006 and 0008.

CREATE TABLE IF NOT EXISTS venue_credentials (
    credential_id   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID NOT NULL REFERENCES users(user_id),
    venue_id        TEXT NOT NULL,
    -- Credentials are stored encrypted (application-layer encryption).
    -- The plaintext is never written to this column.
    ciphertext      BYTEA NOT NULL,
    key_version     TEXT NOT NULL DEFAULT 'v1',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, venue_id)
);

CREATE INDEX IF NOT EXISTS venue_credentials_user ON venue_credentials(user_id);
