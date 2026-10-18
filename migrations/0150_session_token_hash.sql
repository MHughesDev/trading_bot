-- 0150 — sessions store a SHA-256 of the bearer token, never the token (Set E 1.1),
--        and venue credentials gain the envelope columns their store needs (Set E 1.2).
--
-- ── Why ─────────────────────────────────────────────────────────────────────
--
-- `sessions.token` (migration 0013) held the bearer credential in cleartext, as
-- the primary key. A database dump, a replica, a logged row or a backup was a
-- full-authority credential for every logged-in user, including
-- `POST /api/trading/resume`.
--
-- ── Shape (ADR-E-1.1, recorded in docs/plans/plan-sets/set-E/phase-1.md) ────
--
-- The column keeps its name and becomes the lowercase hex SHA-256 of the
-- presented token. The rejected alternative was the spec's literal
-- `token_hash BYTEA` column: a rename breaks, at runtime and silently, every
-- query in the crate that reads `sessions` by `token = $1` with the key the
-- `BearerToken` extractor hands it (strategy authorship, research-session
-- attribution, the agent audit trail). Keeping the name and putting the key —
-- not the credential — in `BearerToken::token` keeps all of them correct with
-- no edit.
--
-- The database, not the application, does the hashing:
--
--   * `session_token_hash(text)` is the ONE definition of the key.
--   * A BEFORE INSERT trigger replaces whatever was inserted with its hash. A
--     writer cannot store cleartext even by mistake: every INSERT supplies the
--     credential and the row holds only the digest.
--   * The key can never be rewritten (an UPDATE that changes it is refused),
--     so nothing can later put cleartext back.
--   * A CHECK pins the stored shape to 64 hex characters.
--
-- Lookups compare `token = session_token_hash($1)`. Comparison is on a digest of
-- a 244-bit random value, so a timing difference in the index probe reveals
-- nothing an attacker can steer.

CREATE OR REPLACE FUNCTION session_token_hash(presented TEXT) RETURNS TEXT
    LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE
AS $$ SELECT encode(sha256(convert_to(presented, 'UTF8')), 'hex') $$;

COMMENT ON FUNCTION session_token_hash(TEXT) IS
    'The sessions.token key for a presented bearer token (Set E 1.1). The only definition.';

-- Expired rows carry no authority; drop them rather than hash them.
DELETE FROM sessions WHERE expires_at <= now();

-- Hash every live token in place. Runs before the triggers below exist, so it
-- is the one UPDATE that may change the key.
UPDATE sessions SET token = session_token_hash(token);

ALTER TABLE sessions
    ADD CONSTRAINT sessions_token_is_a_sha256_hex CHECK (token ~ '^[0-9a-f]{64}$');

-- Sliding expiry (Set E 1.1): a web session's expiry moves forward while it is
-- used, up to an absolute cap; `last_seen_at` is what the touch throttles on.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now();

-- The sweep deletes by expiry.
CREATE INDEX IF NOT EXISTS sessions_expires_at ON sessions (expires_at);

-- Web logins hold full user rights (0038). Login and register inserted rows with
-- the column default `{}` after 0038 ran, so every web session minted since then
-- held no scope at all and failed every `permits` check (the kill switch among
-- them). The application now inserts `web:full`; existing rows are repaired here.
UPDATE sessions SET scopes = ARRAY['web:full'] WHERE kind = 'web' AND scopes = '{}';

CREATE OR REPLACE FUNCTION sessions_store_only_the_hash() RETURNS trigger
    LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF NEW.token IS NULL OR NEW.token = '' THEN
            RAISE EXCEPTION 'a session needs a token' USING ERRCODE = 'not_null_violation';
        END IF;
        NEW.token := session_token_hash(NEW.token);
        RETURN NEW;
    END IF;
    -- UPDATE
    IF NEW.token IS DISTINCT FROM OLD.token THEN
        RAISE EXCEPTION 'sessions.token is a digest and is never rewritten'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS sessions_store_only_the_hash_trg ON sessions;
CREATE TRIGGER sessions_store_only_the_hash_trg
    BEFORE INSERT OR UPDATE ON sessions
    FOR EACH ROW EXECUTE FUNCTION sessions_store_only_the_hash();

GRANT SELECT, INSERT, UPDATE, DELETE ON sessions TO app_role;
GRANT EXECUTE ON FUNCTION session_token_hash(TEXT) TO app_role;

-- ── Venue credentials (Set E 1.2) ────────────────────────────────────────────
--
-- 0007 created the table with a single `ciphertext` column, but the envelope
-- scheme in `credentials/crypto.rs` produces a nonce and a KEK-wrapped DEK as
-- well; without them a stored credential could never be decrypted. Nothing has
-- ever written this table (there was no route), so the new columns are added
-- NOT NULL with no backfill concern beyond an empty table.
ALTER TABLE venue_credentials ADD COLUMN IF NOT EXISTS nonce BYTEA;
ALTER TABLE venue_credentials ADD COLUMN IF NOT EXISTS wrapped_dek BYTEA;
ALTER TABLE venue_credentials ADD COLUMN IF NOT EXISTS envelope_key_version INTEGER;
-- Field names only (e.g. {api_key, api_secret}) — never values — so the UI can
-- say what is stored without anything being decrypted.
ALTER TABLE venue_credentials ADD COLUMN IF NOT EXISTS field_names TEXT[] NOT NULL DEFAULT '{}';
ALTER TABLE venue_credentials ADD COLUMN IF NOT EXISTS key_last4 TEXT;

DELETE FROM venue_credentials WHERE nonce IS NULL OR wrapped_dek IS NULL;
ALTER TABLE venue_credentials ALTER COLUMN nonce SET NOT NULL;
ALTER TABLE venue_credentials ALTER COLUMN wrapped_dek SET NOT NULL;
ALTER TABLE venue_credentials ALTER COLUMN envelope_key_version SET NOT NULL;

GRANT SELECT, INSERT, UPDATE, DELETE ON venue_credentials TO app_role;
