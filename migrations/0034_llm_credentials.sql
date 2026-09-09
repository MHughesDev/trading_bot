-- Per-user LLM provider credentials (OpenAI / Anthropic / local Ollama) for
-- the internal agent. API keys are AES-256-GCM envelope-encrypted with the
-- operator KEK (env CRED_KEK) — see crates/api/src/credentials/crypto.rs.
-- Columns match EncryptedCredential { ciphertext, nonce, wrapped_dek,
-- key_version } exactly (deliberately not the older 0007 venue shape).

CREATE TABLE IF NOT EXISTS llm_credentials (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    provider    TEXT NOT NULL CHECK (provider IN ('openai', 'anthropic', 'ollama')),
    ciphertext  BYTEA NOT NULL,   -- encrypted API key (empty plaintext allowed for ollama)
    nonce       BYTEA NOT NULL,
    wrapped_dek BYTEA NOT NULL,   -- DEK nonce prepended, per crypto.rs
    key_version INT   NOT NULL DEFAULT 1,
    base_url    TEXT,             -- endpoint override (Ollama host); NULL = provider default
    key_last4   TEXT,             -- display only, never the key
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, provider)
);
